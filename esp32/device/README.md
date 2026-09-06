# The first silicon number

`budget.py` divides 4,266 measured operations by a documented clock and brackets
cycles-per-operation at 1.0, 2.5 and 5.0 because nobody had run it. This
directory runs the same two loops on a real Xtensa and reports what they cost.

**Measured 6 Sep 2026, NodeMCU (ESP8266EX, Xtensa LX106) at 80 MHz, -O2:**

| | value |
|---|---|
| median cycles per message | **188,576** |
| **cycles per operation** | **44.50** |
| per message | 2,357 µs |
| throughput | 424 messages/s |
| agreement with `slots.c` | **141 of 141 messages, 0 disagreements** |

Reproduced identically on three consecutive runs. The correctness column is the
one that licenses the timing column: a board that classifies differently than the
trainer is not measuring this model, so all 23 accumulators are compared against
`slots.c` on every message before a cycle is reported.

## Read the 44.50 carefully

**It does not refute the 1.0–5.0 bracket, because it is not the same regime.**
The bracket describes an ESP32-S3 with the 92 KiB of weights in internal SRAM.
This is an ESP8266 with 48,952 bytes of free heap, so the weights live in flash
and every one of the ~3,300 weight reads per message goes through
`pgm_read_byte` and a 32 KiB cache that 92 KiB of randomly-indexed buckets
thrash. What is being measured here is arithmetic plus a cache miss, and the
miss dominates.

That is worth having rather than a disappointment, because it puts a number on a
claim the parent README makes and could not previously support:

> At 92 KiB the weights are 18% of the internal SRAM, so the PSRAM bandwidth
> wall never applies — which is the whole reason the last row is four orders of
> magnitude from the others rather than merely smaller.

SRAM residency was the load-bearing assumption in that sentence and it had never
been priced. On a part where the weights do **not** fit, the identical code costs
**44.50 cycles/op instead of an assumed 1.0–5.0** — an order of magnitude, paid
entirely for where the bytes sit. The argument survives; it now has evidence
under it.

## What is built and not yet measured

The experiment is designed to separate arithmetic from memory, and only half of
it has run. **The operation count does not move with the bucket width** — it is
n-grams × (2 per hashed byte + 23 accumulates) — so the same work can be run
against weights in flash and against weights in DRAM with nothing else changing,
and the ratio is the memory term alone.

At 4,096 buckets the DRAM half cannot run: 92 KiB does not fit in 49 KiB of
heap, and the sketch reports `dram_weights=no`. A 1,024-bucket build (23 KiB,
`slots.bin.1024`) fits and is compiled and ready; it has not been flashed
because the board is behind a USB2.0 hub whose bulk transfers corrupt the
bootloader protocol — `read_flash` failed at 57% and `write_flash` failed six
times with `Timed out waiting for packet header` and truncated register reads.
The running sketch's short request/response lines are unaffected, which is why
the numbers above are trustworthy and the reflash is not. **Plug the board
directly into the host, not through a hub, and the DRAM row completes.**

Until it does, this directory measures one regime and says so.

## Running it

```bash
cd esp32
python3 extract_msgs.py > msgs.txt
OTHER=0 python3 gen_slots.py 1200 train.jsonl
python3 train_slots.py 4096            # -> slots.bin
cc -O2 -o slots slots.c

cd device
python3 weights_header.py              # slots.bin -> slots_device/weights.h
pio run                                # firmware.bin
python3 device_check.py /dev/cu.usbserial-XXX
```

`device_check.py` exits non-zero on any disagreement and needs no `pyserial`:
115200 is a standard rate, so `termios` reaches it without the macOS-only
`IOSSIOSPEED` ioctl, and this directory stays at numpy plus a C compiler like
the rest of `esp32/`.

## Two things that cost an hour each, recorded so they do not again

**There is no arm64 macOS toolchain for the ESP8266.** The Arduino core's stable
release and PlatformIO both ship x86_64 `xtensa-lx106-elf-gcc`, and this host has
no Rosetta. The build in `platformio.ini` therefore runs inside an **arm64 Linux
container**, where the `linux_aarch64` toolchain is native — no emulation, no
admin rights. Flashing still happens from the host, because `esptool.py` is
Python and runs natively.

**The harness reset the board and then flushed the port.** The ROM bootloader
talks at 74880 baud, so its banner arrives at 115200 as bytes with no line
structure; flushing after the release leaves a fragment of it in the buffer,
where it is read as the head of the first reply. The symptom is not a parse
error on message 0 — it is a silent one-line offset that surfaces a hundred
messages later as `MALFORMED reply: 'READY'`, which reads exactly like a board
that keeps rebooting and is not. `Port.reset()` flushes while reset is held.

## The guards, and that they can fail

* **Accumulators, not argmax** — the reason `slots.c` dumps them. This went red
  for all 141 messages when a 1,024-bucket `slots.bin` was checked against a
  4,096-bucket firmware, which is what a real fold divergence would look like.
* **The op model is checked, not asserted.** `ops_for()` re-derives the work from
  the folded length and every cycles/op figure divides by it, so a wrong model
  rescales the whole table silently. It is compared against `slots.c`'s own
  count over the same 131 benchmark messages: both say **4,266**. Changing the
  per-n-gram term from 23 to 22 turns it red.
* **The device compares its own two paths.** A `pgm_read_byte` returning the
  wrong byte for an odd offset is invisible to a host compiler; when the DRAM
  path is present the sketch memcmps the two accumulator sets and reports
  `FLASH_RAM_DIFF`. On this build the DRAM path is absent, so this guard is
  declared and not yet exercised.
