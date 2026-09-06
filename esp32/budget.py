"""What fits on an ESP32-S3, and what the arithmetic says it would cost.

THIS IS ARITHMETIC, NOT SILICON, and it stays that way: no ESP32-S3 has run it.
What IS measured is the operation count (slots.c counts its own ops) and the
model size (bytes on disk); the ESP32-S3 figures are those two numbers divided by
documented characteristics of that part, and every assumption is printed next to
its result so a reader with a board can check it rather than trust it.

A READER WITH A BOARD DID. device/ runs the same loops on an ESP8266 and measures
44.50 cycles/op against the 1.0/2.5/5.0 guessed below - nine times the
pessimistic row, because there the 92 KiB does not fit in SRAM and lives in flash
instead. That does not correct these numbers, which describe a part where it
does; it prices the assumption they rest on. See device/README.md.
"""
import json, os, subprocess

CLOCK_HZ   = 240_000_000     # ESP32-S3, dual Xtensa LX7, one core used
SRAM_BYTES = 512 * 1024      # internal SRAM
FLASH_BYTES = 16 * 1024 * 1024
PSRAM_BW   = 40e6            # octal PSRAM, conservative sustained bytes/s

def measure_ops():
    out = subprocess.run(["./slots", "500"], stdin=open("msgs.txt"),
                         capture_output=True, text=True).stdout
    ops = size = mean = count = None
    for line in out.splitlines():
        if "ops per inference" in line:
            ops = float(line.split("=")[-1])
        if "weights " in line and "bytes" in line:
            size = int(line.split()[1])
        if "mean" in line:
            # "  messages           131, mean 48.9 bytes"
            count = int(line.split()[1].rstrip(","))
            mean = float(line.split("mean")[1].split()[0])
    return ops, size, mean, count

ops, weights, mean_len, count = measure_ops()

# THE COUNT COMES FROM THE RUN, NOT FROM A LITERAL. This line said "on the 36
# benchmark messages" while reading 131 of them, so a reader who re-ran the
# script to check the page got a different number with nothing to explain it.
print(f"MEASURED (by slots.c, on the {count} benchmark messages)")
print(f"  weights                {weights:,} bytes ({weights/1024:.1f} KiB), int8")
print(f"  mean message           {mean_len:.0f} bytes")
print(f"  ops per inference      {ops:,.0f}   (FNV hash + int8 accumulate)")
print()
print("ESP32-S3 BUDGET (arithmetic from the two numbers above)")
print(f"  internal SRAM          {SRAM_BYTES//1024} KiB")
print(f"  weights fit in SRAM    {'yes' if weights < SRAM_BYTES else 'no'}"
      f"  — {100*weights/SRAM_BYTES:.0f}% of it, so no PSRAM and no bandwidth wall")
print(f"  accumulators           23 int32 = 92 bytes")
print(f"  fold buffer            1 KiB")
print()
for cpo, why in ((1.0, "optimistic: everything single-cycle"),
                 (2.5, "likely: int8 load + add, no SIMD"),
                 (5.0, "pessimistic: cache misses and loop overhead")):
    us = ops * cpo / CLOCK_HZ * 1e6
    print(f"  at {cpo:>3.1f} cycles/op       {us:6.1f} us per message   ({why})")
print()

print("WHAT A GENERATIVE MODEL WOULD COST INSTEAD")
print("  A decode step reads every weight once, so tokens/s <= bandwidth / size.")
# ONE BANDWIDTH FOR BOTH ROWS, AND THE LABEL SAYS SO. The first model is small
# enough to live in flash and the rest are not, so strictly the flash row should
# be rated at quad-SPI flash rather than at PSRAM. It is rated at PSRAM anyway,
# and that is defensible rather than sloppy: 40 MB/s meets or exceeds sustained
# quad-SPI on the modelled part, flash and PSRAM share SPI0 and the same cache on
# an S3, and the number therefore stays a true CEILING — loose in the generative
# model's favour, which is the direction an argument against it must be loose in.
print(f"  PSRAM sustained ~{PSRAM_BW/1e6:.0f} MB/s; the flash row is rated at the")
print("  same figure, which is generous to it — see the note in this script.\n")
row = "  {:<28} {:>10} {:>9} {:>14}"
print(row.format("model", "Q4 weights", "fits?", "ceiling"))
for name, params in (("TinyStories-15M", 15e6),
                     ("SmolLM2-135M", 135e6),
                     ("FunctionGemma-270M", 270e6),
                     ("Qwen3-0.6B", 600e6)):
    size = params * 0.5                       # 4-bit
    fits = "flash" if size < FLASH_BYTES else "no"
    toks = PSRAM_BW / size
    ceiling = f"{toks:.2f} tok/s" if toks >= 0.01 else "< 0.01 tok/s"
    print(row.format(name, f"{size/1e6:.0f} MB", fits, ceiling))
print()
print(f"  the slot classifier          {weights/1024:.0f} KiB     SRAM   "
      f"{1e6/(ops*2.5/CLOCK_HZ*1e6):>8,.0f} msg/s")
