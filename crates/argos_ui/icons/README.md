# Icons

`icon.png` is the source of record: 1024×1024 with a transparent surround,
which is the size the macOS `.icns` needs. Everything beside it is generated
from it by `tauri icon icons/icon.png` and read by the bundler through the
`bundle.icon` list in `tauri.conf.json`. Nothing here is edited by hand.

The artwork was drawn on a wider canvas, on black, with a glow around the
drive; what is committed is the drive alone, on transparency, so the icon has
no background of its own and sits on any desktop.

Brightness cannot separate the two — the shadowed quarter of the platter reads
91 and the glow just outside the rim reads 87 — and neither can one gradient
threshold. The platter's rim falls 122 levels across six pixels, but the
actuator housing sits at 76 against a glow of 53, a step of twenty; a cut
placed to keep the housing keeps the glow with it.

What separates them is that the glow is smooth *everywhere* while every part of
the drive has an outline. Measured on this artwork, touching pixels differ by
at most 1 anywhere in the glow, while the shallowest edge on the drive steps 3.
So the background is grown from the border one pixel at a time, crossing only
where the level changes by no more than 2: it flows through the glow and stops
at the drive. What the growth does not reach is the icon, less anything the
ramp stranded — the drive is one object, so only the largest piece is kept.

The remaining hairline of glow clinging to the outline is taken by a pixel of
erosion, and a slight blur gives the edge back the anti-aliasing that squared
off.

`tauri icon` rewrites `icon.png` itself, at 512×512. The 1024×1024 source is
put back afterwards: the `.icns` it generated does carry the large sizes
(`ic10`, `ic14`), but the file the next run would read must not be the
downscale of the one this run read.

The mobile sets, the Windows Store logos and the 64×64 that command also emits
are deleted: no bundle target reads them (`A-EARNS-ITS-PLACE`).
