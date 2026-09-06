/* Slot inference as it would run on an ESP32-S3: int8 weights, int32
 * accumulators, no float, no malloc, no libm.
 *
 * WHAT IS BEING COUNTED. Two loops and nothing else: FNV-1a over each character
 * n-gram, and one int8 add per (n-gram, class). The op counter is incremented in
 * the same places the device would execute, so the number reported is the
 * device's work rather than this laptop's.
 */
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <stdlib.h>
#include <time.h>

#define NHEADS 6
static const int HEAD_CLASSES[NHEADS] = {2, 3, 5, 4, 4, 5};
static const char *HEAD_NAME[NHEADS] = {"gate","tool","audience","price","when","intent"};

static uint32_t BUCKETS;
static int8_t *W[NHEADS];          /* [BUCKETS][classes], row-major */

static uint64_t ops_hash, ops_acc;

/* THE SAME FOLD THE TRAINER USES, and it has to be exactly the same or the
 * device hashes different n-grams than the ones the weights were fitted to.
 * Lowercase ASCII, and map the six Turkish letters (two-byte UTF-8) onto their
 * bare forms. `fold_matches_python` in check.py asserts the two agree on every
 * benchmark message; without that check this function can drift silently and
 * the only symptom is an accuracy nobody can explain.
 */
static size_t fold(const uint8_t *in, size_t n, uint8_t *out)
{
    size_t j = 0;
    out[j++] = ' ';
    int space = 1;
    for (size_t i = 0; i < n; ) {
        uint8_t c = in[i];
        uint8_t m = 0;
        if (c < 0x80) {
            m = (c >= 'A' && c <= 'Z') ? (uint8_t)(c + 32) : c;
            i += 1;
        } else {
            /* UTF-8 LENGTH FROM THE LEAD BYTE. Assuming two bytes for every
             * non-ASCII character reads an em dash as a Turkish letter and then
             * resumes mid-sequence — the trainer kept its three bytes, the
             * device kept garbage, and the n-gram counts differed by four. Only
             * the two-byte Turkish letters are mapped; every other sequence is
             * skipped whole, and the trainer drops the same characters. */
            int seq = (c >= 0xF0) ? 4 : (c >= 0xE0) ? 3 : (c >= 0xC0) ? 2 : 1;
            if (i + seq > n) break;
            if (seq != 2) { i += seq; continue; }
            uint16_t w = (uint16_t)(c << 8 | in[i + 1]);
            switch (w) {
            case 0xC4B1: case 0xC4B0: m = 'i'; break;   /* ı  İ */
            case 0xC49F: case 0xC49E: m = 'g'; break;   /* ğ  Ğ */
            case 0xC3BC: case 0xC39C: m = 'u'; break;   /* ü  Ü */
            case 0xC59F: case 0xC59E: m = 's'; break;   /* ş  Ş */
            case 0xC3B6: case 0xC396: m = 'o'; break;   /* ö  Ö */
            case 0xC3A7: case 0xC387: m = 'c'; break;   /* ç  Ç */
            case 0xC3A2: case 0xC382: m = 'a'; break;   /* â  Â */
            case 0xC3AE: case 0xC38E: m = 'i'; break;   /* î  Î */
            default: m = 0; break;
            }
            i += 2;
            if (!m) continue;                            /* drop what we cannot fold */
        }

        /* EVERY ASCII WHITESPACE, INCLUDING THE FOUR SEPARATORS. Python's
         * str.split also breaks on \x1c-\x1f (file, group, record, unit), which
         * pasted EDI, CSV and SMS content really does carry. Stopping at \f gave
         * the same n-gram COUNT and different bytes — invisible to a length
         * check, and enough to flip an argmax. */
        if (m == ' ' || m == '\t' || m == '\n' || m == '\r' || m == '\v' || m == '\f'
            || m == 0x1c || m == 0x1d || m == 0x1e || m == 0x1f) {
            if (!space) { out[j++] = ' '; space = 1; }
        }
        else { out[j++] = m; space = 0; }
    }
    if (!space) out[j++] = ' ';
    else if (j > 1 && out[j-1] != ' ') out[j++] = ' ';
    return j;
}

/* One inference. `acc` must hold at least max(classes) int32s per head. */
static void infer(const uint8_t *s, size_t len, int32_t acc[NHEADS][8], uint32_t *ngrams)
{
    for (int h = 0; h < NHEADS; h++)
        for (int c = 0; c < HEAD_CLASSES[h]; c++) acc[h][c] = 0;
    *ngrams = 0;

    for (int n = 3; n <= 5; n++) {
        if (len < (size_t)n) continue;
        for (size_t i = 0; i + n <= len; i++) {
            uint32_t hsh = 2166136261u;
            for (int k = 0; k < n; k++) {
                hsh = (hsh ^ s[i + k]) * 16777619u;   /* xor + imul */
                ops_hash += 2;
            }
            uint32_t b = hsh % BUCKETS;
            (*ngrams)++;
            for (int h = 0; h < NHEADS; h++) {
                const int8_t *row = W[h] + (size_t)b * HEAD_CLASSES[h];
                for (int c = 0; c < HEAD_CLASSES[h]; c++) {
                    acc[h][c] += row[c];              /* the whole model */
                    ops_acc++;
                }
            }
        }
    }
}

int main(int argc, char **argv)
{
    FILE *f = fopen("slots.bin", "rb");
    if (!f) { perror("slots.bin"); return 1; }
    uint32_t nheads;
    if (fread(&BUCKETS, 4, 1, f) != 1 || fread(&nheads, 4, 1, f) != 1) return 1;
    size_t total = 0;
    for (int h = 0; h < NHEADS; h++) {
        uint32_t nc, pad;
        if (fread(&nc, 4, 1, f) != 1 || fread(&pad, 4, 1, f) != 1) return 1;
        if ((int)nc != HEAD_CLASSES[h]) { fprintf(stderr, "head %d: %u classes\n", h, nc); return 1; }
        size_t bytes = (size_t)BUCKETS * nc;
        W[h] = malloc(bytes);
        if (fread(W[h], 1, bytes, f) != bytes) return 1;
        total += bytes;
    }
    fclose(f);

    /* The messages to classify come in on stdin, one per line. */
    static char line[512];
    int32_t acc[NHEADS][8];
    uint32_t ngrams = 0, count = 0;
    size_t chars = 0;

    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    int reps = argc > 1 ? atoi(argv[1]) : 1;
    static char buf[4096][512];
    int nbuf = 0;
    /* ONE MESSAGE PER LINE, WITH ESCAPES. A message that contains a newline is
     * exactly the case the fold got wrong, and it cannot travel over a
     * line-oriented stdin as itself. check.py writes \n \r \t \v \f as
     * backslash escapes and this undoes them, so the bytes folded here are the
     * bytes the trainer folded. */
    while (nbuf < 4096 && fgets(line, sizeof line, stdin)) {
        size_t l = strlen(line);
        while (l && (line[l-1] == '\n' || line[l-1] == '\r')) line[--l] = 0;
        char *d = buf[nbuf];
        for (size_t i = 0; i < l; i++) {
            if (line[i] == '\\' && i + 1 < l) {
                char n = line[++i];
                *d++ = n == 'n' ? '\n' : n == 'r' ? '\r' : n == 't' ? '\t'
                     : n == 'v' ? '\v' : n == 'f' ? '\f' : n == '\\' ? '\\' : n;
            } else {
                *d++ = line[i];
            }
        }
        *d = 0;
        nbuf++;
    }
    for (int r = 0; r < reps; r++)
        for (int i = 0; i < nbuf; i++) {
            static uint8_t folded[1024];
            size_t l = fold((const uint8_t *)buf[i], strlen(buf[i]), folded);
            infer(folded, l, acc, &ngrams);
            if (r == 0) { chars += l; count++; }
            if (r == 0 && getenv("DUMP")) {
                /* THE ACCUMULATORS, NOT THE ARGMAX. Dumping only the winning
                 * class made the cross-check unable to fail: breaking the fold
                 * so that `ı` stops folding to `i` changes the features of every
                 * Turkish message and did not flip a single argmax on the 36
                 * benchmark cases. A guard that survives its own defect is worth
                 * nothing, so what is compared is every accumulator — where a
                 * one-letter difference cannot hide. */
                /* THE INDEX, NOT THE TEXT. Keying the dump on the message
                 * itself means a message containing a newline or a tab breaks
                 * the line-oriented output that carries it — the harness fails
                 * where the fold is fine, which is the worst kind of red. The
                 * order is deterministic on both sides. */
                printf("%d\t%u", i, ngrams);
                for (int h = 0; h < NHEADS; h++)
                    for (int c = 0; c < HEAD_CLASSES[h]; c++)
                        printf("\t%d", acc[h][c]);
                printf("\n");
            }
            if (r == 0 && count <= 3 && !getenv("DUMP")) {
                printf("  \"%.44s\" ->", buf[i]);
                for (int h = 0; h < NHEADS; h++) {
                    int best = 0;
                    for (int c = 1; c < HEAD_CLASSES[h]; c++)
                        if (acc[h][c] > acc[h][best]) best = c;
                    printf(" %s=%d", HEAD_NAME[h], best);
                }
                printf("\n");
            }
        }
    clock_gettime(CLOCK_MONOTONIC, &t1);
    double secs = (t1.tv_sec - t0.tv_sec) + (t1.tv_nsec - t0.tv_nsec) / 1e9;
    uint64_t infers = (uint64_t)nbuf * reps;

    printf("\n  weights            %zu bytes (%.1f KiB), %u buckets\n", total, total/1024.0, BUCKETS);
    printf("  messages           %u, mean %.1f bytes\n", count, (double)chars/count);
    printf("  inferences         %llu in %.3f s\n", (unsigned long long)infers, secs);
    printf("  per inference      %.1f us on this host\n", secs*1e6/infers);
    printf("  ops per inference  %.0f hash + %.0f accumulate = %.0f\n",
           (double)ops_hash/infers, (double)ops_acc/infers,
           (double)(ops_hash+ops_acc)/infers);
    return 0;
}
