/* The slot classifier on real silicon.
 *
 * WHY THIS EXISTS. esp32/README.md quotes 17.8 / 44.4 / 88.9 us per message at
 * 1.0 / 2.5 / 5.0 cycles per operation and says plainly that those are
 * "arithmetic, not silicon" - 4,266 measured operations divided by a documented
 * clock, with the cycles/op figure bracketed three ways because nobody had run
 * it. This sketch runs the same two loops on an Xtensa and reports the cycle
 * count, so that column can be measured instead of guessed.
 *
 * THE BOARD IS AN ESP8266, NOT THE ESP32-S3 THE BUDGET MODELS. That is a real
 * limitation and it is not hidden: LX106 at 80 MHz against LX7 at 240 MHz, and
 * a 32 KiB cache in front of flash against 512 KiB of internal SRAM. What
 * survives the difference is cycles per operation for THIS code on THIS
 * instruction set family, which is the only quantity budget.py guesses.
 * device/README.md states what the number does and does not license.
 *
 * TWO MEMORY REGIMES, ONE OPERATION COUNT. The op count does not move with the
 * bucket width - it is n-grams x (2 per hashed byte + 23 accumulates) - so the
 * same work can run against weights in flash and against weights in DRAM with
 * nothing else changing. That difference is exactly what separates budget.py's
 * optimistic row from its pessimistic one.
 *
 * THE DRAM PATH IS ALSO THE CORRECTNESS CONTROL. A flash byte read on this part
 * goes through pgm_read_byte because the mapped flash window faults on an
 * unaligned load, and a host compiler cannot expose that bug class at all.
 * Running both paths over every message and comparing all 23 accumulators makes
 * such a fault a visible disagreement rather than an accuracy nobody explains.
 */
#include <Arduino.h>
#include <ESP8266WiFi.h>
#include "weights.h"

#define NHEADS 6
static const uint8_t HEAD_CLASSES[NHEADS] = {2, 3, 5, 4, 4, 5};
#define NCLASS 23                       /* 2+3+5+4+4+5, the dump width */

/* REPETITIONS, AND THE MINIMUM OF THEM. The mean is not the quantity wanted:
 * the SDK's timer tick lands inside some runs and inflates them, so a mean
 * measures the scheduler as much as the loop. The minimum over a handful of
 * identical runs is the run that was not interrupted, which is the cost of the
 * code. The radio is powered down below for the same reason. */
#define REPS 5

static int8_t *WRAM = nullptr;          /* null when the blob does not fit */

/* ---------------------------------------------------------------- the fold
 * COPIED FROM slots.c, BYTE FOR BYTE, and it has to stay that way: a hashed
 * feature model cannot complain when two folds disagree, it just scores worse.
 * check.py asserts C against Python; device_check.py asserts this against the C.
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
            int seq = (c >= 0xF0) ? 4 : (c >= 0xE0) ? 3 : (c >= 0xC0) ? 2 : 1;
            if (i + seq > n) break;
            if (seq != 2) { i += seq; continue; }
            uint16_t w = (uint16_t)(c << 8 | in[i + 1]);
            switch (w) {
            case 0xC4B1: case 0xC4B0: m = 'i'; break;
            case 0xC49F: case 0xC49E: m = 'g'; break;
            case 0xC3BC: case 0xC39C: m = 'u'; break;
            case 0xC59F: case 0xC59E: m = 's'; break;
            case 0xC3B6: case 0xC396: m = 'o'; break;
            case 0xC3A7: case 0xC387: m = 'c'; break;
            case 0xC3A2: case 0xC382: m = 'a'; break;
            case 0xC3AE: case 0xC38E: m = 'i'; break;
            default: m = 0; break;
            }
            i += 2;
            if (!m) continue;
        }
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

/* ------------------------------------------------------------- inference
 * TWO FUNCTIONS FROM ONE MACRO, rather than one function with a flag. A runtime
 * branch on the weight source sits inside the innermost loop - 3,300 executions
 * per message - so it would be measured along with the memory it exists to
 * distinguish, which defeats the experiment.
 */
#define DEFINE_INFER(NAME, READ)                                               \
static uint32_t NAME(const uint8_t *s, size_t len, int32_t *acc)               \
{                                                                              \
    for (int i = 0; i < NCLASS; i++) acc[i] = 0;                              \
    uint32_t ngrams = 0;                                                       \
    for (int n = 3; n <= 5; n++) {                                             \
        if (len < (size_t)n) continue;                                         \
        for (size_t i = 0; i + n <= len; i++) {                                \
            uint32_t hsh = 2166136261u;                                        \
            for (int k = 0; k < n; k++)                                        \
                hsh = (hsh ^ s[i + k]) * 16777619u;                            \
            uint32_t b = hsh % W_BUCKETS;                                      \
            ngrams++;                                                          \
            int32_t *a = acc;                                                  \
            for (int h = 0; h < NHEADS; h++) {                                 \
                uint32_t off = HEAD_OFF[h] + b * HEAD_CLASSES[h];              \
                for (int c = 0; c < HEAD_CLASSES[h]; c++)                      \
                    *a++ += (int32_t)(int8_t)READ(off + c);                    \
            }                                                                  \
        }                                                                      \
    }                                                                          \
    return ngrams;                                                             \
}

#define READ_FLASH(o) pgm_read_byte(&WEIGHTS[(o)])
#define READ_RAM(o)   WRAM[(o)]
DEFINE_INFER(infer_flash, READ_FLASH)
DEFINE_INFER(infer_ram,   READ_RAM)

/* ------------------------------------------------------------------- I/O */
static char line[600];
static uint8_t raw[512];
static uint8_t folded[1024];
static int32_t acc_f[NCLASS], acc_r[NCLASS];

/* slots.c's escaping, reversed. A message with a newline is exactly the case the
 * fold once got wrong and cannot travel over a line-oriented link as itself. */
static size_t unescape(const char *in, uint8_t *out, size_t cap)
{
    size_t j = 0;
    for (size_t i = 0; in[i] && j + 1 < cap; i++) {
        if (in[i] == '\\' && in[i + 1]) {
            char n = in[++i];
            out[j++] = n == 'n' ? '\n' : n == 'r' ? '\r' : n == 't' ? '\t'
                     : n == 'v' ? '\v' : n == 'f' ? '\f' : n == '\\' ? '\\' : (uint8_t)n;
        } else out[j++] = (uint8_t)in[i];
    }
    return j;
}

void setup()
{
    /* 128 BYTES IS THE DEFAULT AND THE MESSAGES ARE LONGER. The escaped form of
     * a pasted multi-line message runs past 400 bytes; the UART ISR fills the
     * ring whether or not loop() is inside readBytesUntil, so the tail of a long
     * line is dropped before it is read and the reply then differs from the C on
     * exactly the messages this harness exists to check. */
    Serial.setRxBufferSize(1024);
    Serial.begin(115200);
    Serial.setTimeout(3000);

    /* THE RADIO IS THE LARGEST SOURCE OF INTERRUPTS ON THIS PART and it has no
     * job here. Left on, its beacon handling lands inside timed regions and the
     * spread across repetitions is wider than the effect being measured. */
    WiFi.mode(WIFI_OFF);
    WiFi.forceSleepBegin();
    delay(20);

    WRAM = (int8_t *)malloc(W_BYTES);
    if (WRAM) memcpy_P(WRAM, WEIGHTS, W_BYTES);

    Serial.println();
    Serial.printf("# chip=esp8266 cpu_mhz=%u buckets=%u weight_bytes=%u\n",
                  ESP.getCpuFreqMHz(), (unsigned)W_BUCKETS, (unsigned)W_BYTES);
    Serial.printf("# dram_weights=%s free_heap=%u reps=%u\n",
                  WRAM ? "yes" : "no", ESP.getFreeHeap(), REPS);
    Serial.println("READY");
}

void loop()
{
    if (!Serial.available()) return;
    size_t n = Serial.readBytesUntil('\n', line, sizeof(line) - 1);
    line[n] = 0;
    while (n && (line[n - 1] == '\r' || line[n - 1] == '\n')) line[--n] = 0;
    if (!n) return;

    if (!strcmp(line, "#END")) { Serial.println("DONE"); return; }

    /* index TAB text, so the reply can be keyed the way check.py keys the C. */
    char *tab = strchr(line, '\t');
    if (!tab) { Serial.println("ERR no index"); return; }
    *tab = 0;
    int idx = atoi(line);

    size_t rl = unescape(tab + 1, raw, sizeof(raw));
    size_t fl = fold(raw, rl, folded);

    uint32_t best_f = 0xFFFFFFFFu, best_r = 0xFFFFFFFFu, ngrams = 0;
    for (int r = 0; r < REPS; r++) {
        uint32_t t0 = ESP.getCycleCount();
        ngrams = infer_flash(folded, fl, acc_f);
        uint32_t t1 = ESP.getCycleCount();
        if (t1 - t0 < best_f) best_f = t1 - t0;      /* unsigned: wrap is fine */
    }
    if (WRAM) {
        for (int r = 0; r < REPS; r++) {
            uint32_t t0 = ESP.getCycleCount();
            infer_ram(folded, fl, acc_r);
            uint32_t t1 = ESP.getCycleCount();
            if (t1 - t0 < best_r) best_r = t1 - t0;
        }
    }

    /* THE ACCUMULATORS, NOT THE ARGMAX - the same reason slots.c dumps them:
     * breaking the fold so a Turkish letter stops folding changed the features
     * of every Turkish message and flipped no argmax at all. */
    Serial.printf("%d\t%u", idx, (unsigned)ngrams);
    for (int i = 0; i < NCLASS; i++) Serial.printf("\t%ld", (long)acc_f[i]);
    Serial.printf("\t%u\t", (unsigned)best_f);
    if (WRAM) {
        int same = memcmp(acc_f, acc_r, sizeof acc_f) == 0;
        Serial.printf("%u\t%s\n", (unsigned)best_r, same ? "ok" : "FLASH_RAM_DIFF");
    } else {
        Serial.println("na\tna");
    }
}
