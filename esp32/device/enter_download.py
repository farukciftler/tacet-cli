"""Put the board in download mode, because esptool's own reset does not here.

WHAT FAILS. `esptool --before default_reset` could not reach the bootloader on
this NodeMCU at all: six attempts, every one dying in `sync()` or `get_chip_id()`
on a short packet, and `flash_id` 0 for 6. It reads like a broken cable and is
not.

WHY. The auto-reset circuit is driven by DTR and RTS *in opposition* — DTR low
with RTS high resets, DTR high with RTS low releases reset while GPIO0 is held
low, and whenever the two are EQUAL both outputs idle high and nothing happens.
esptool changes them with two separate ioctls, so between the first and the
second the pair passes through an equal state; on a CH340 behind a hub that
window is long enough for the chip to leave reset with GPIO0 still high, which
is a normal boot. The sketch then answers esptool's sync bytes with its own
output, and esptool reports serial corruption.

Setting both lines in ONE TIOCMSET removes the intermediate state. Measured by
reading the ROM's own banner at 74880 baud, which names the mode it chose:

    two ioctls, esptool's order   ->  (nothing, or boot mode:(3,6) = flash)
    one ioctl, this file          ->  boot mode:(1,6) = download

AND THEN CLOSE THE PORT, which is safe: leaving DTR and RTS both deasserted
leaves both outputs high, the idle state, so the chip stays in download mode
while esptool opens the port for itself. Hand it `--before no_reset` so it does
not undo this.

    python3 enter_download.py /dev/cu.usbserial-XXX
    esptool.py --chip esp8266 --port /dev/cu.usbserial-XXX --before no_reset \
               --no-stub write_flash --flash_mode dio --flash_size detect 0x0 \
               .pio/build/nodemcuv2/firmware.bin
"""
import os, sys, fcntl, termios, struct, time

TIOCM_DTR, TIOCM_RTS = 0x002, 0x004


def enter(port):
    fd = os.open(port, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
    try:
        a = termios.tcgetattr(fd)
        a[0] = a[1] = a[3] = 0
        # HUPCL IS NOT SET, deliberately: with it, closing the port drops DTR and
        # the pulse that produces is another reset, out of the mode just entered.
        a[2] = termios.CS8 | termios.CREAD | termios.CLOCAL
        a[6][termios.VMIN] = 0
        a[6][termios.VTIME] = 0
        termios.tcsetattr(fd, termios.TCSANOW, a)

        def mctrl(dtr, rts):
            cur = struct.unpack('I', fcntl.ioctl(fd, termios.TIOCMGET, struct.pack('I', 0)))[0]
            cur = (cur | TIOCM_DTR) if dtr else (cur & ~TIOCM_DTR)
            cur = (cur | TIOCM_RTS) if rts else (cur & ~TIOCM_RTS)
            fcntl.ioctl(fd, termios.TIOCMSET, struct.pack('I', cur))

        mctrl(False, True);  time.sleep(0.12)   # RST low
        mctrl(True,  False); time.sleep(0.08)   # GPIO0 low, RST released
        mctrl(False, False)                     # both idle high; mode is latched
    finally:
        os.close(fd)


if __name__ == "__main__":
    enter(sys.argv[1] if len(sys.argv) > 1 else "/dev/cu.usbserial-140")
    print("download mode")
