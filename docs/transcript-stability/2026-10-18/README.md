# Transcript stability — 2026-10-18

The transcript reserved four speculative lines inside streamed prose and another
four below commentary. Starting tools and ending a run removed those reserves,
so bottom-follow moved the existing content. The short-answer baseline shrank
from 103.2 px to 25.8 px at completion; finishing work after a completed tool
removed 115.2 px of loading geometry.

The current UI renders prose at its actual height and keeps one fixed line for
activity, including idle. New assistant rows fade without translating over
command receipts. The obsolete placeholder estimator and its contracts were removed.

`node harness/rendering-stability.mjs` passed Chromium and WebKit with normal
and reduced motion. Status changes and work completion caused 0 px geometry
shift; short replies retained their height. Twelve commands kept distinct DOM
rows and titles across rapid appends and output disclosure animation, with
0 px overlap across 58–71 sampled frames per configuration. All twelve were
visible at the end. Disclosure, scrollback, and code-fence retention checks passed.

[Validation and source digests](validation.json) retain the four configurations.
[Recording](after.webm) and [screenshot](after.png) show a synthetic local QA
conversation. The phone-access banner belongs to the fixture account. No live
provider or private user conversation was used. Installed native and packaged
acceptance remain separate from these browser checks.
