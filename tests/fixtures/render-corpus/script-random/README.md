# Original random-sequence contract

The four original scripts use headers 2.11, 2.19, 2.21 and 2.19; the final node changes dimensions and inputs. Each samples zero-based draws 0–31, 63, 255, 1023, 4095 and 65535. Two 24-bit RGB cells encode each 48-bit fractional value; white panel gaps remain part of the comparison.

The independent Rust oracle evaluates unsigned Mulberry32 arithmetic from seed zero, then encodes those words directly. Tests check all 37 samples per script, per-execution resets, worker output, and exact RGBA equality for both document schemas at scale two. The original CC0 dedication is retained in resources/licenses/script-random/NOTICE.txt. No vendor PNG or decoded observation is used. See the parent corpus README for permission/history status.
