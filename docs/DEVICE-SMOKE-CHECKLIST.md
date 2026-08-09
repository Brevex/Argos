# Device HAL smoke checklist

The automated suite covers everything that can be checked without a disk: the
path conventions, the class and TRIM decisions, the mount-table parsing, and
that an image-file scan recovers the same bytes on all three operating
systems. What it cannot cover is the part where a real kernel answers a real
ioctl, or where a real desktop's elevation prompt hands back a process that
can still be spoken to.

**Every value below is unverified against hardware until someone runs this and
records the result.** That is not a formality: the request codes in
`crates/argos_device/src/device/macos.rs` are computed from Darwin's `_IOR`
encoding rather than read back from a running system, and the Windows
descriptors are filled by a storage driver whose behaviour varies by vendor.
The failure mode of a wrong constant is a wrong device class or a refused
open, not a wrong recovery — but wrong is wrong, and this is where it gets
caught.

Run once per operating system, per release. Record the date, the OS build, the
hardware, and the observed output in the table at the bottom.

---

## Before you start

- Use a disk you are willing to lose. Argos never writes to a source
  (`A-READ-ONLY`), but this checklist has you attach media and elevate, and a
  mistyped path in some *other* tool is a real risk on a day you are doing
  this kind of work.
- Prefer a small USB stick or an external enclosure over an internal system
  disk. Two media are better than one: one spinning, one solid-state, so the
  class detection has something to be right or wrong about.
- Have a second, non-Argos way of reading the same facts to compare against —
  `lsblk`/`smartctl`, `diskutil info`, `Get-PhysicalDisk`. The point of the
  exercise is the comparison, not Argos' own output.

---

## 1. Enumeration without privileges

Run as an ordinary user, **not** elevated:

```
argos devices
```

- [ ] Exits successfully.
- [ ] Every whole disk on the machine is listed, before any partition.
- [ ] Each partition appears under a disk it really belongs to.
- [ ] Sizes match the OS's own tools, byte for byte where the OS reports
      bytes.
- [ ] The class (`hdd` / `ssd` / `unknown`) matches what the OS says. **A
      wrong class here is a bug; `unknown` is not** — a driver that declines
      the query is expected and correct.
- [ ] On Windows specifically, check the class against a USB-attached disk as
      well as an internal one. The seek-penalty and TRIM descriptors are
      filled by whichever driver owns the stack, and USB-SATA bridges are the
      ones that answer oddly — including the non-zero-but-not-one `BOOLEAN`
      values the reply parsing is written to tolerate.
- [ ] Where TRIM is reported, it matches the OS.
- [ ] Mounted media show their mount points, and read-only mounts are marked.

Compare against:

| OS | Command |
| --- | --- |
| Linux | `lsblk -o NAME,SIZE,ROTA,MODEL,MOUNTPOINTS` |
| macOS | `diskutil list` and `diskutil info diskN` |
| Windows | `Get-PhysicalDisk \| Format-List` and `Get-Partition` |

## 2. Opening a device read-only

Elevated (`sudo`, or an Administrator shell). Use a **whole disk**, and one
that is not the system disk:

```
argos scan <whole-disk-path> --out ./smoke-out
```

- [ ] The scan opens the device and reports its size, matching step 1.
- [ ] If the medium is mounted, the mount warning appears and names the mount
      points.
- [ ] If the medium is solid-state with TRIM, the reduced-expectation note
      appears.
- [ ] The scan runs to completion without an I/O error on a healthy disk.
- [ ] `manifest.json` is written and its `source` names the device.

Path form per OS:

| OS | Whole disk | Partition (for step 3) |
| --- | --- | --- |
| Linux | `/dev/sdX`, `/dev/nvmeXnY` | `/dev/sdX1`, `/dev/nvmeXnYp1` |
| macOS | `/dev/disk4` | `/dev/disk4s1` |
| Windows | `\\.\PhysicalDrive2` | — see note |

macOS additionally needs **Full Disk Access** for the terminal application, in
System Settings → Privacy & Security. Without it the open fails with a
permission error rather than anything more informative; that is the OS, not
Argos.

## 3. The partition warning

Point a scan at a **partition** rather than the disk:

```
argos scan <partition-path> --out ./smoke-part
```

- [ ] The warning about scanning one partition appears.
- [ ] The reported size is the partition's, not the disk's.

This is the warning that stops someone reporting "nothing was recoverable"
after scanning 500 MB of a 2 TB disk.

## 4. Read-only, verified from outside

The claim that Argos never writes to a source is structural — no write path
exists in `argos_device` — but it is worth confirming from outside the program
once per platform.

- [ ] Take a hash of the whole device before the scan and after it, and
      confirm they match. On Linux/macOS: `sudo shasum -a 256 /dev/rdiskN`
      (this reads the whole disk; use a small medium).
- [ ] On Windows, confirm in Process Monitor that the `argos.exe` handle to
      `\Device\HarddiskN` has read access only.

## 5. Shadow copies (Windows only)

On a system with System Protection enabled and at least one restore point:

```
argos devices
```

- [ ] The shadow copies section lists at least one snapshot.
- [ ] The count matches `vssadmin list shadows` (elevated).
- [ ] Scanning one of the listed shadow paths opens and reads it. This one
      has never been exercised: a shadow copy is a *volume* handle, so the
      length and geometry ioctls are being asked of something other than a
      physical drive, and whether they answer usefully is exactly what this
      step establishes.

Recall what this does **not** report: a snapshot's creation time and
originating volume come from the VSS COM API, which Argos does not use. If the
count or the paths look wrong, that is a bug; if the timestamps are absent,
that is the documented boundary.

## 6. The shell, against a real device

The window never runs elevated. It spawns `argos serve` and speaks JSON-RPC to
it, and asking for privileges is only a choice about how that child is
started. Which means this step is about one thing: whether the elevated child
can still be talked to.

- [ ] Launch the shell, tick "raise privileges", and start the engine. On
      Linux the desktop's authentication agent should prompt, and the engine
      should connect afterwards.
- [ ] **Windows and macOS are expected to refuse here, with an explanation.**
      Both elevate through a shell verb — `ShellExecuteW` with `runas`,
      `osascript … with administrator privileges` — that does not give the
      caller the child's pipes, so there is nothing to speak JSON-RPC over.
      What must be true is that the refusal *says so* and points at
      `argos scan` from an elevated terminal. A hang, a silent failure, or an
      engine that connects and then reports an empty medium because it is
      running unprivileged are all bugs.
- [ ] Without elevation, scanning a raw **image file** works in the shell on
      all three platforms. This is the path that does not depend on any of the
      above.
- [ ] With a scan running, open the gear and switch themes. Nothing should be
      interrupted: the rings keep moving, the figures keep counting, and the
      warnings stay on screen. A theme switch rewrites custom properties and
      remounts nothing, so anything lost here is a bug.
- [ ] Watch the two rings against a real medium. **Scan** should climb steadily
      while **Recovery**, "Images recovered" and "Data recovered" stay at zero
      until the sweep finishes — this pipeline validates after it sweeps, and
      those figures count artifacts actually written, never candidates seen. A
      count that moves during the sweep would mean something is reporting
      unvalidated hits as recoveries.
- [ ] Stop a scan partway with the button and confirm the destination folder
      holds a `manifest.json` describing exactly the files beside it.

## 7. Bad-sector handling

Optional, and only with a medium that already has read errors — do not create
them.

- [ ] Unreadable regions appear in the manifest's `unreadable` list.
- [ ] The scan continues past them rather than aborting.
- [ ] No artifact overlaps an unreadable range.

---

## Results

One row per run. An empty table means this has never been executed, which is
the honest state until it has.

| Date | OS and build | Hardware | Steps passed | Notes / deviations |
| --- | --- | --- | --- | --- |
| | | | | |
