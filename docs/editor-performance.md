# Editor responsiveness

Verified on 2026-11-10 with optimized macOS builds and isolated synthetic data.

## Changes

- Preserve TextKit layout by applying only changed attribute runs. Build the desired formatting off-screen and apply it synchronously before the input event returns.
- Reuse native checkbox and list-marker views, bullet images, block previews and loaded attachments. Update action offsets and geometry when text moves.
- Reuse attribute dictionaries; traverse formatting ranges in order instead of repeatedly scanning every marker.
- Cache tag, thumbnail and search metadata by source text. Reuse the editor's tag analysis where its parsing rules match the sidebar.
- Limit list snippets to 240 Unicode scalar values. The underlying note and full-text search remain intact.
- Buffer archive writes while retaining flush, file synchronization and atomic replacement.

## Measurements

The harness loads 100 notes, sends 35 characters followed by 35 backspaces at 15 ms intervals, verifies that the typed string was saved, then checks exact equality with the original text. The times below measure native key handling, including immediate formatting; they are not end-to-end screen presentation times.

| Active note | Median key handling | 95th percentile |
| --- | ---: | ---: |
| 200 checklist items | 4.6 ms | 6.6 ms |
| 200 bullet items | 4.3 ms | 6.4 ms |
| 200 numbered items | 4.3 ms | 6.1 ms |
| 200 paragraphs repeating one formula | 8.9 ms | 9.4 ms |
| 10 image blocks | 2.3 ms | 3.9 ms |
| 20 groups of tables, callouts and separators | 2.3 ms | 5.1 ms |
| 1,000 formatted paragraphs | 16.9 ms | 17.8 ms |

Before optimization, the 200-item checklist took approximately 72 ms per key. The ordinary 200-paragraph scenario took approximately 17 ms per key plus 15 ms building the workspace. Image-note workspace construction fell from approximately 19 ms to below 1 ms. Buffered saves in these scenarios take approximately 7–10 ms rather than 42 ms.

Measured event-queue wait stays below approximately 20 ms for the checklist and list scenarios. The 1,000-paragraph stress test reaches approximately 55 ms at the deliberately aggressive input rate. Initial document loading and first-time math rendering are excluded from steady-state key timings and can take longer. Markdown parsing still scales with the size of the open note.

## Reproduce

Build an optimized bundle, then run from the repository root:

```sh
packaging/bundle.sh release
packaging/profile-editor.sh "$PWD/target/Polar.app" 200 tasks
packaging/profile-editor.sh "$PWD/target/Polar.app" 200 numbers
packaging/profile-editor.sh "$PWD/target/Polar.app" 200 math
packaging/profile-editor.sh "$PWD/target/Polar.app" 10 images
packaging/profile-editor.sh "$PWD/target/Polar.app" 20 blocks
packaging/profile-editor.sh "$PWD/target/Polar.app" 1000 plain
packaging/profile-editor.sh "$PWD/target/Polar.app" 200 tasks @todo
```

The harness requires macOS GUI automation permissions, Swift and ImageMagick for the image fixture. It creates a uniquely identified test app and scratch notes, targets only that app, and preserves its log and data in the printed temporary directory. It fails if the editor does not receive all insertions/deletions or if the final note differs from the original. `POLAR_PROFILE=1` enables diagnostic timings; normal operation does not emit them.

## Correctness gates

`cargo fmt --check`, strict Clippy and 22 unit tests pass. Native editor smoke tests also pass in an optimized build with debug assertions enabled. They cover immediate font sizes, source reveal, formulas, separators, tables, images, callouts, checkbox actions, undo, reading/protected modes, overlays and Unicode offsets. Additional regressions verify that checkbox instances survive typing, cached tags/thumbnails/search features update after edits, caches do not enter saved archives, and snippets do not truncate the source note.

## Distinct-formula regression

A subsequent investigation found that the previous math cache cleared all entries at 128 keys. Notes with more than 128 distinct formulas could therefore rerender most formulas on every editing event. The earlier formula benchmark repeated one expression and did not exercise this failure.

The cache now retains the current document's formula set and releases removed formulas. A native regression test first failed with 180 distinct formulas, then passed after the fix. An editor test performs 35 insertions and 35 deletions and asserts that the math renderer's call count does not increase and the source is restored exactly. This native test completed in approximately 66 ms total in the optimized build; this is not an end-to-end GUI latency measurement. Cache pruning is also tested.

The GUI harness now supports `math_unique`, for example:

```sh
packaging/profile-editor.sh "$PWD/target/Polar.app" 180 math_unique
```

The GUI attempts for this follow-up stopped at the foreground-app safety check, so no GUI timing is claimed for that scenario. The native regression and full smoke suite passed.
