# Evidence handling — full rule text

Argos-specific rules (`A-*` namespace). Same conventions as the `M-*` rules: one rule lives in
exactly one file, reviewers cite ids. Index: [../../guidelines-index.md](../../guidelines-index.md).

## No write path to a source medium exists (A-READ-ONLY) { #A-READ-ONLY }

<why>the source medium is evidence; a single stray write can destroy it irrecoverably.</why>

Devices and images under analysis are opened read-only at the lowest layer (`O_RDONLY`,
`GENERIC_READ`), and the abstraction over them — the `BlockSource` port — exposes no write,
truncate, discard or ioctl-passthrough method. There is deliberately nothing to review for
write-safety, because the capability does not exist in the type system.

- The only writes Argos performs are to user-chosen destinations: acquired images, recovered
  artifacts, manifests, logs. Destination paths must be validated to not resolve onto the source
  device.
- A safe-but-destructive operation (opening writable "just to lock", issuing a discard, updating
  an on-device timestamp) is a critical finding regardless of memory safety.
- Test code is not exempt: tests never open a real device node writable, and `test-util`
  fixtures are in-memory or plain files.

## On-disk values are untrusted input (A-UNTRUSTED-ONDISK) { #A-UNTRUSTED-ONDISK }

<why>disk images are attacker-controllable and routinely corrupt; trusting them turns corruption
into crashes, OOMs or wrong answers.</why>

Any value decoded from the medium — length, offset, count, cluster number, chain pointer, flag —
must be treated as hostile until validated:

- **Arithmetic**: `checked_add`/`checked_mul`/`checked_sub` (or `saturating_*` where the semantics
  allow), never bare operators. Overflow on untrusted math is a recoverable parse error, not a bug.
- **Indexing**: `slice.get(n)` / `get(a..b)`, never `slice[n]`, whenever `n` derives from the
  medium. An out-of-range access is corruption, i.e. a `Result::Err`.
- **Allocation**: never size an allocation from an on-disk value alone. Cap with a documented
  named constant (`M-DOCUMENTED-MAGIC`) chosen from format limits, and reject or stream anything
  larger.
- **Loops and chains**: any structure that can self-reference (FAT chains, extent trees, MFT
  attribute lists) gets an explicit iteration bound derived from the medium's size, so a crafted
  cycle terminates as a parse error.
- **Fabrication**: clamping or defaulting an invalid value and continuing silently fabricates
  evidence. Fail the parse of that object; the scan continues past it.

## Every artifact carries provenance (A-PROVENANCE) { #A-PROVENANCE }

<why>a recovery that cannot be traced back to sectors is not auditable and has no evidentiary
value.</why>

Each recovered artifact records, at minimum: the recovery stage that produced it, the ordered list
of source extents (LBA ranges) its bytes came from, the source filesystem object (inode/MFT record)
when one exists, timestamps recovered from metadata (never invented), and a SHA-256 computed at the
moment of recovery. Reassembled artifacts additionally record every candidate extent used, so the
reconstruction is reproducible from the same image.

## Confidence is reported at the evidence tier, never above (A-CONFIDENCE-HONEST) { #A-CONFIDENCE-HONEST }

<why>an examiner acts on the tool's word; optimism is indistinguishable from fabrication.</why>

The confidence tier ladder is fixed:
`FsMetadata > JournalResidue > ContiguousCarve > Reassembled > PartialOrThumbnail`.

- An artifact's tier is determined by the weakest link in how it was obtained, and no
  post-processing (successful decode, ML score) may raise it.
- Unreadable sectors are reported as unreadable — never zero-filled and presented as read.
- TRIM-erased and overwritten ranges are reported as unrecoverable; the tool states reduced
  expectations for TRIMmed SSDs up front.
- Partial recoveries state their truncation plainly (bytes recovered vs expected).

## Logs never contain recovered content (A-NO-CONTENT-IN-LOGS) { #A-NO-CONTENT-IN-LOGS }

<why>recovered data is someone's private material; telemetry must not become a second leak.</why>

Extends `M-LOG-STRUCTURED`: no log line, structured event, metric label, error `Display`, or panic
message may include recovered file content, filenames or paths recovered from the medium, or EXIF
fields. Permitted: LBAs, byte offsets, sizes, counts, hashes, format names, error kinds. The rule
covers test output too — fixture content is synthetic precisely so that accidental logging of it
stays harmless, but the discipline is identical.
