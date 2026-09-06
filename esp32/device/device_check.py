"""The board must compute what the C computes, and this is what it costs there.

TWO JOBS, AND THE FIRST GATES THE SECOND. A timing figure from a device that
classifies differently than the trainer measures nothing, so all 23 accumulators
are compared against slots.c on every message before a single cycle count is
reported. This is check.py's contract carried one hop further out: Python vs C
vs Rust there, C vs silicon here.

WHY THE CYCLES MATTER. esp32/README.md's device table is 4,266 operations
divided by a clock, with cycles-per-operation bracketed at 1.0 / 2.5 / 5.0
because it had never been run. The op count does not move with the bucket width,
so the same work measured against weights in flash and against weights in DRAM
isolates memory from arithmetic - which is the difference between that table's
optimistic row and its pessimistic one.

NO pyserial. 115200 is a standard rate, so termios reaches it without the
macOS-only IOSSIOSPEED ioctl, and this directory stays at numpy plus a C
compiler the way the rest of esp32/ is.
"""
import os, sys, fcntl, termios, struct, time, select, subprocess, statistics
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)) + "/..")
from train_slots import HEADS, fold, read_benchmark
from shapes import SHAPES

NCLASS = sum(len(v) for v in HEADS.values())
TIOCM_DTR, TIOCM_RTS = 0x002, 0x004


def escape(t):
    """slots.c's transport, and the sketch reverses the same set."""
    return (t.replace("\\", "\\\\").replace("\n", "\\n").replace("\r", "\\r")
             .replace("\t", "\\t").replace("\v", "\\v").replace("\f", "\\f"))


class Port:
    def __init__(self, path, baud=termios.B115200):
        self.fd = os.open(path, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
        a = termios.tcgetattr(self.fd)
        a[0] = a[1] = a[3] = 0                       # raw in, raw out, no echo
        a[2] = termios.CS8 | termios.CREAD | termios.CLOCAL
        a[4] = a[5] = baud
        a[6][termios.VMIN] = 0
        a[6][termios.VTIME] = 0
        termios.tcsetattr(self.fd, termios.TCSANOW, a)
        self.buf = b""

    def reset(self):
        """RST is driven from RTS on this board and GPIO0 from DTR; GPIO0 high
        boots from flash. Resetting rather than trusting whatever state the last
        run left means the banner read below belongs to this run."""
        self._mctrl(False, True); time.sleep(0.1)
        # FLUSH WHILE RESET IS HELD, NOT AFTER IT IS RELEASED. The ROM bootloader
        # talks at 74880 baud, so its banner arrives here as bytes with no line
        # structure at all; flushing after the release either eats the sketch's
        # own banner or leaves a fragment of that garbage in the buffer, where it
        # is read as the head of the first reply. The symptom is not a parse
        # error on message 0 but a silent one-line offset that surfaces a hundred
        # messages later as "MALFORMED reply: 'READY'", which reads like a board
        # that keeps rebooting and is not.
        termios.tcflush(self.fd, termios.TCIFLUSH)
        self.buf = b""
        self._mctrl(False, False)

    def _mctrl(self, dtr, rts):
        cur = struct.unpack('I', fcntl.ioctl(self.fd, termios.TIOCMGET, struct.pack('I', 0)))[0]
        cur = (cur | TIOCM_DTR) if dtr else (cur & ~TIOCM_DTR)
        cur = (cur | TIOCM_RTS) if rts else (cur & ~TIOCM_RTS)
        fcntl.ioctl(self.fd, termios.TIOCMSET, struct.pack('I', cur))

    def line(self, timeout=6.0):
        end = time.time() + timeout
        while b"\n" not in self.buf:
            if time.time() > end:
                return None
            r, _, _ = select.select([self.fd], [], [], 0.1)
            if r:
                try:
                    self.buf += os.read(self.fd, 4096)
                except BlockingIOError:
                    pass
        l, self.buf = self.buf.split(b"\n", 1)
        return l.decode("utf-8", "replace").rstrip("\r")

    def write(self, s):
        b = s.encode()
        while b:
            try:
                b = b[os.write(self.fd, b):]
            except BlockingIOError:
                time.sleep(0.002)


def host_reference(msgs):
    """slots.c's accumulators, keyed by index, from the binary check.py uses."""
    here = os.path.dirname(os.path.abspath(__file__))
    out = subprocess.run(["./slots", "1"], cwd=here + "/..",
                         input="\n".join(escape(m) for m in msgs) + "\n",
                         capture_output=True, text=True,
                         env=dict(os.environ, DUMP="1"))
    if out.returncode != 0:
        sys.exit("./slots failed - build it with `cc -O2 -o slots slots.c`\n" + out.stderr[-400:])
    ref = {}
    for line in out.stdout.splitlines():
        p = line.split("\t")
        if len(p) == 2 + NCLASS:
            ref[int(p[0])] = (int(p[1]), [int(x) for x in p[2:]])
    if not ref:
        sys.exit("./slots produced no DUMP rows")
    return ref


def ops_for(text):
    """The operation count slots.c would report for ONE message.

    DERIVED, NOT READ BACK. slots.c prints the mean over its whole input; the
    cycles here are per message, so dividing by a mean would attribute the long
    messages' cycles to the short messages' work. Both terms are exact: FNV does
    two operations per byte of each n-gram, and every n-gram touches all 23
    weights once. The device's own n-gram count is asserted against the second
    term below, so a wrong model of the work cannot pass quietly."""
    L = len(fold(text))
    grams = [max(0, L - n + 1) for n in (3, 4, 5)]
    return sum(2 * n * g for n, g in zip((3, 4, 5), grams)) + NCLASS * sum(grams), sum(grams)


def main():
    port_path = sys.argv[1] if len(sys.argv) > 1 else "/dev/cu.usbserial-140"
    rows = read_benchmark()
    msgs = [r["text"] for r in rows] + list(SHAPES)
    ref = host_reference(msgs)

    p = Port(port_path)
    p.reset()
    banner = []
    while True:
        l = p.line(timeout=8)
        if l is None:
            sys.exit(f"no READY from {port_path} - is slots_device flashed?")
        if l.startswith("#"):
            banner.append(l)
        if l.strip() == "READY":
            break
    for b in banner:
        print(b)

    bad, flash_c, dram_c, tot_ops = 0, [], [], []
    for i, m in enumerate(msgs):
        p.write(f"{i}\t{escape(m)}\n")
        rep = p.line(timeout=10)
        if rep is None:
            print(f"NO REPLY for message {i}: {m[:44]!r}"); bad += 1; continue
        f = rep.split("\t")
        if len(f) != 2 + NCLASS + 3:
            print(f"MALFORMED reply for {i}: {rep[:120]!r}"); bad += 1; continue
        idx, ngrams = int(f[0]), int(f[1])
        acc = [int(x) for x in f[2:2 + NCLASS]]
        cyc_f, cyc_r, agree = f[2 + NCLASS], f[3 + NCLASS], f[4 + NCLASS]
        if idx != i:
            print(f"INDEX {idx} for message {i}"); bad += 1; continue
        want_ngrams, want_acc = ref[i]
        if ngrams != want_ngrams:
            print(f"NGRAM COUNT: {m[:44]!r} c {want_ngrams} vs device {ngrams}")
            bad += 1; continue
        if acc != want_acc:
            d = [k for k, (a, b) in enumerate(zip(want_acc, acc)) if a != b]
            print(f"ACCUMULATORS: {m[:44]!r} differ in {len(d)} of {NCLASS}")
            bad += 1; continue
        # THE BOARD'S OWN FLASH-VS-DRAM COMPARISON. A pgm_read_byte returning the
        # wrong byte for an odd offset is invisible to a host compiler and would
        # otherwise surface only as a score nobody can account for.
        if agree not in ("ok", "na"):
            print(f"FLASH/DRAM DISAGREE on device: {m[:44]!r}"); bad += 1; continue
        ops, grams = ops_for(m)
        if grams != ngrams:
            print(f"OPS MODEL: {m[:44]!r} predicts {grams} n-grams, device counted {ngrams}")
            bad += 1; continue
        tot_ops.append(ops)
        flash_c.append(int(cyc_f))
        if agree == "ok":
            dram_c.append(int(cyc_r))

    p.write("#END\n")
    p.line(timeout=3)

    print(f"\n{len(msgs)} messages, {bad} disagreements between the device and the C")
    if bad or not flash_c:
        return 1

    mhz = next((int(b.split("cpu_mhz=")[1].split()[0]) for b in banner if "cpu_mhz=" in b), 80)
    print(f"\nMEASURED ON SILICON ({len(flash_c)} messages, min of 5 runs each, {mhz} MHz)")
    hdr = "  {:<22} {:>12} {:>11} {:>10} {:>12}"
    print(hdr.format("weights in", "median cyc", "cycles/op", "us/msg", "msg/s"))
    for name, cyc in (("flash (pgm_read_byte)", flash_c), ("DRAM", dram_c)):
        if not cyc:
            print(hdr.format(name, "-", "-", "-", "did not fit"))
            continue
        per_op = [c / o for c, o in zip(cyc, tot_ops)]
        med, mop = statistics.median(cyc), statistics.median(per_op)
        us = med / (mhz * 1e6) * 1e6
        print(hdr.format(name, f"{med:,.0f}", f"{mop:.2f}", f"{us:.0f}", f"{1e6/us:,.0f}"))
    # THE OP MODEL IS CHECKED, NOT ASSERTED. ops_for() re-derives the work from
    # the folded length, and every cycles/op figure above divides by it, so a
    # wrong model would rescale the whole table silently. slots.c counts its own
    # operations while executing them; over the SAME set the two must agree.
    # This line first read "slots.c reports the same figure over the same set"
    # beside a mean taken over 141 messages while slots.c had counted 131 - the
    # ten fold shapes are shorter than the benchmark and pulled it to 4,128
    # against 4,266. The claim was true of a set that was not the one printed.
    n_bench = len(rows)
    model_bench = statistics.mean(tot_ops[:n_bench])
    c_ops = None
    here = os.path.dirname(os.path.abspath(__file__))
    try:
        with open(here + "/../msgs.txt", "rb") as f:
            out = subprocess.run(["./slots", "1"], cwd=here + "/..", stdin=f,
                                 capture_output=True, text=True).stdout
        for line in out.splitlines():
            if "ops per inference" in line:
                c_ops = float(line.split("=")[-1])
    except FileNotFoundError:
        pass
    print(f"\n  mean ops/message       {model_bench:,.0f} over the {n_bench} benchmark"
          f" messages, {statistics.mean(tot_ops):,.0f} over all {len(tot_ops)}")
    if c_ops is None:
        print("  slots.c op count       not available (msgs.txt missing)")
    elif abs(c_ops - model_bench) > 1:
        print(f"  MISMATCH               slots.c counted {c_ops:,.0f} on the same"
              f" {n_bench}; the op model above is wrong")
        return 1
    else:
        print(f"  slots.c counted        {c_ops:,.0f} on the same {n_bench} - the op model agrees")
    if dram_c:
        rf = statistics.median(flash_c) / statistics.median(dram_c)
        print(f"  flash / DRAM           {rf:.2f}x - the memory term budget.py brackets")
    return 0


if __name__ == "__main__":
    sys.exit(main())
