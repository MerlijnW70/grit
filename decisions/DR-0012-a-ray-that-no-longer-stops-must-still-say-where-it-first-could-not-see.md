---
id: DR-0012
title: A ray that no longer stops must still say where it first could not see
status: prose-only
---

## The Measurement

In `src/bin/vista.rs` a ray writes the number of a chunk the card does not hold into a buffer, and
the host fetches what that buffer names. While a missing chunk ended the ray, the number written was
the only one there was. When a missing chunk stopped ending the ray — so that what is behind it is
drawn instead of a wall — each further missing chunk the ray crossed overwrote the last, and what
came back was **the furthest chunk of every ray rather than the nearest**.

The same fixed flight of 2400 frames, the two builds differing in that one line: the share of the
frame that came back as plain sky, and so as ground the eye should have seen and did not, is 62.1%
writing the last against 22.7% writing the first, with 5888 chunks landed against 6721 and 1.76
GiB/s sustained against 1.94. The 22.7% is the sky above the horizon.

## The Decision

The report is written once a ray, on the first chunk it could not see through, and a chunk the card
does not hold is crossed exactly as an empty one is — reported, and stepped over.

## The Rejected Route

Drawing a missing chunk as a colour of its own is rejected. It is what a demonstration wants, in that
it shows the loop working, and it is what put walls across the picture whenever the eye moved: flown
by hand, 2.804% of the last frame was that colour, in blocks large enough to read as scenery. What is
behind an absent chunk is wrong until it lands and right the moment it does, which is a better answer
than a wall that is wrong either way.

## The Limit

The share of the frame left as sky is a property of the flight and the scene as much as of the
report — the two runs above are the same fixed flight over the same world and are comparable to each
other, and to nothing else. Whether reporting more than one chunk a ray would fill the picture sooner
is NOT MEASURED: the buffer holds one number a ray by construction. The apparatus was a demonstration that was never part of the crate, and it has been removed: `src/bin/vista.rs` and the world it read are gone, so nothing here can be re-run as it stands. What is above is the record of what it measured while it existed.
