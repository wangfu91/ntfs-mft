# Raw MFT implementation

Start with the [API guide](api-guide.md) for scan selection and error contracts,
or the [stream guide](offline-mft.md) for per-slot parsing. The
[module map](../src/raw_mft/README.md) locates the implementation.

## Bootstrap and reads

`RawMft::new` reads NTFS boot geometry and FILE record zero, discovers the
unnamed `$MFT::$DATA` runlist, builds an `ExtentMap`, and loads the MFT allocation
bitmap when available. Sector, cluster, and record sizes come from boot geometry;
1 KiB records are common but not required. ReFS is rejected.

The read path is boot sector → MFT runlist → extent map → FILE record → USA
fixup → attributes. Extent lookup maps logical record numbers across fragmented
storage and sparse holes, checks complete record coverage, and caches the
segment index. Chunk boundaries are logical ranges, independent of physical extents.

`VolumeReader` buffers aligned reads. Windows uses explicit-offset `ReadFile`
with a per-reader completion event; Linux/macOS use positional reads, and macOS
queries device block alignment. Contiguous records borrow buffered bytes;
records spanning extents use a reusable scratch buffer. A separate reader serves
random attribute lookups without disturbing sequential read-ahead.

`Volume` retains its source for reopening. Linux resolves mounts through
`/proc/self/mountinfo`; macOS uses `statfs`. Workers and owned byte streams reopen
independent read-only handles. Platform requirements are documented for
[Windows](windows-raw-volume-access.md) and [macOS](macos-raw-volume-access.md).

## Record parsing and enrichment

Serial scanning and parallel workers share the scan engine:

1. Check allocation evidence when unused records are excluded.
2. Translate the logical record number, skipping sparse holes.
3. Read the record, validate the FILE header, and apply USA fixups.
4. Skip extension records before building a base entry.
5. Build metadata and selectively enrich it through `$ATTRIBUTE_LIST`.

The entry builder collects standard information, names, data, reparse tags,
and attribute-list references. Primary-name selection prefers `Win32AndDos`,
then `Win32`, `Posix`, and `Dos`. Options control shadowed DOS aliases and ADS.
Unnamed DATA supplies primary size/storage metadata; named DATA supplies ADS.

Enrichment runs when a base entry needs a better name, missing file data, or a
reparse tag. It reads resident or nonresident lists and merges usable attributes
from referenced extension records. It follows one level and does not emit
extensions as separate files. Rich serial and parallel scans share this logic;
lean scans omit rich-only metadata, and cached enrichment is lean-only.

`RawMftEntry::metadata()` describes observed presence and disabled collection
without reparsing or allocating diagnostics. Missing fields remain unknown and
completeness is unverified. Use `MftStream` for its supported inspection evidence.

## Chunk planning

Automatic planning and execution use the same scan options. Partition sizing
defaults to at most 16,384 logical records per chunk. The planner retains bounded
bands containing any used record, omitting wholly unused bands unless requested.
Missing bitmap evidence keeps candidates eligible.

The shared allocator checks the candidate count before bitmap filtering and
reserves fallibly. Each plan is bounded to 64 MiB of descriptors; worker buffers,
entries, cached extensions, and collected results require additional memory.
Explicit chunks are clipped to selection and volume bounds, then checked for
overlap using a sorted copy while retaining their supplied execution order.
See [selection and planning](api-guide.md#selection-and-planning) for public
limits, errors, and builder behavior.

## Parallel execution

Each worker owns sequential and attribute readers. Boot geometry is cloned;
the extent map and bitmap are shared through `Arc`. Workers perform both I/O
and parsing. One worker uses serial chunk processing.

- `Dynamic` workers claim the next chunk through a shared atomic index.
- `Contiguous` workers receive fixed bands of chunk indices.

Workers can finish out of order. The coordinator buffers completed results
and visits them in supplied chunk order. Scanner errors, unexpected channel
closure, and worker panics fail the operation.

Mapping transforms each batch on its worker. Folding builds an accumulator per
chunk without an intermediate entry vector. Lean cached enrichment uses captured
extension records and reads missing dependencies from the volume, including
outside selected ranges. Dependency reads do not add output records.

The [sparse fixture](../tests/fixtures/sparse-volume-fixture/README.md) exercises
bootstrap, fragmented bitmap loading, byte streams, and serial/parallel agreement.
See [contributor guidance](../CONTRIBUTING.md#benchmarks-and-profiling) for
benchmarks and matching profiling targets.
