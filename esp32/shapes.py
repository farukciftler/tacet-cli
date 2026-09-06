"""The message shapes the fold treats specially, in one place.

EXTRACTED SO TWO HARNESSES CANNOT DRIFT. check.py compares the trainer against
the C and device/device_check.py compares the C against the board; both have to
send the same awkward strings or the second one certifies a narrower fold than
the first. The bug that made this list necessary -- a newline kept on one side
and collapsed on the other -- shipped precisely because a check ran on a message
set that could not contain it.

WRITTEN WITH ESCAPES, NOT LITERALS. Two of these rows are one codepoint each and
that codepoint is invisible in an editor: U+00A0 looks like a space and U+212A
looks like a K. Pasting the file through anything that normalises whitespace
silently turns them into ASCII, and the row keeps its comment while testing
nothing -- the same shape of defect as the newline it was written for.
"""

SHAPES = [
    "Şunu yazdı:\n'Cuma günü ödeyeceğim'\nBu ne demek?",
    "line one\r\nline two\r\nline three",
    "tabs\tand\vvertical\fform feeds",
    "  leading and trailing   ",
    "İYİ BAYRAMLAR, ĞÜŞÖÇ upper case",
    "an em \u2014 dash and an emoji \U0001f642 and a nbsp\u00a0here",
    "a",
    # The four ASCII separators. `str.split()` breaks on them; a hand-written
    # fold that stops at \f does not, and pasted EDI, CSV and SMS content really
    # carries them. Same n-gram count, different bytes, different buckets.
    "unit\x1fsep and record\x1esep",
    "group\x1dsep and file\x1csep",
    # U+212A KELVIN SIGN. Python's `str.lower()` maps it to an ASCII `k`, which
    # survives the ASCII filter; C and Rust see three bytes they cannot fold and
    # drop them whole. One codepoint, two feature vectors.
    "temperature 300\u212a today",
]
