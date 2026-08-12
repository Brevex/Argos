# Icons

`icon.png` is the source of record: 1024×1024 with a transparent surround,
which is the size the macOS `.icns` needs. Everything beside it is generated
from it by `tauri icon icons/icon.png` and read by the bundler through the
`bundle.icon` list in `tauri.conf.json`. Nothing here is edited by hand.

The artwork was drawn on a wider canvas with a glow around it; what is
committed is the drive alone, cropped to its rim and masked to a circle, so the
icon has no background of its own and sits equally well on a light or a dark
desktop.

The mobile sets and the Windows Store logos that command also emits are
deleted: no bundle target reads them (`A-EARNS-ITS-PLACE`).
