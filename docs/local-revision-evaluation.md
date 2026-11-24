# Polar local revision prototype

**22 November 2026 · Integrated into the editor.** The signed Swift helper passed all five organization fixtures with source IDs. Correction still missed a typo or rejected a proposal in four of seven fixtures. Both actions are available with explicit preview and confirmation. See the [prototype evaluation](#evaluation-with-source-ids) and the [editor verification](#editor-verification).

## Configuration and method

- Apple M4, 16 GiB unified memory (`Mac16,12`), macOS 27.2.1 build `26B131`.
- Xcode 27.2 build `27B52`, macOS SDK 27.2. The signed helper targets arm64 and macOS 26.4. The machine reported **AFM 3 Core Advanced** and an 8,192-token context.
- Request and response ceilings were 8 KiB and 32 KiB. The client deadline was 30 seconds. Preflight counted the instructions, prompt, and guided schema, reserving 1,536 tokens for a reply and 128 for session overhead. The second round used greedy sampling and examples for Markdown repair.
- Each fixture started a new process inside the signed debug bundle with network access denied to the test process. This exercises the system on-device API with a network-restricted client. The host and macOS model service retained their normal network configuration. The system model was already available.
- Latency is elapsed time from process launch to exit, including startup and generation. Peak resident memory covers the helper process. Each helper was freshly launched. The system model's cold state, service memory, asset size, and host-wide offline relaunch remain unmeasured.

Build with `./packaging/bundle.sh debug` and verify both signatures with `codesign --verify --deep --strict target/Polar.app`. Replay a fixture through the packaged helper using the following command.

```sh
printf '%s' '{"action":"correct","markdown":"Riunone **martedì"}' |
    /usr/bin/sandbox-exec -p '(version 1)(allow default)(deny network*)' \
    target/Polar.app/Contents/Helpers/polar-note-revision
```

## Italian fixtures

| Fixture | Source and required result | Observed after prompt tuning |
| --- | --- | --- |
| C1 | `Ho incontratto Giulia. Il preventivo non è approvato.` Fix `incontratto` and keep the negation. | Correct text. Added an unnecessary notice in English. |
| C2 | `Riunone **martedì`. Fix `Riunone`. A closing `**` is allowed as a formatting suggestion. | Fixed the typo and closed the marker. The notice quoted the revised text, which is absent from the source. |
| C3 | `Da tenere ** testo** nel riepilogo.` Remove the stray space and keep `testo` bold. | Repaired the delimiters, inserted `il`, and quoted the revised sentence in a notice. |
| C4 | `-5 gradi fuori. Il preventivo non e stato approvato.` Fix `e` to `è`. Keep the temperature and negation. | Correct text. The notice quoted a sentence with a corrected accent that was absent from the source. |
| O1 | Four paragraphs alternating between a quote and rooms. Keep Luca sending prices, Giulia being asked about capacity, the 900 euro limit excluding VAT, and `Sale: forse martedì, non confermato.` | Grouped topics and kept the facts. Removed `Sale:` from the affected passage while quoting the full passage in a notice. |
| O2 | Four flat bullets alternating between budget and train. Keep Marta, the train receipt, `Budget: non superare 180 euro`, and `Treno: orario ancora da decidere`. | Duplicated all four points in a nested list. |
| C5, new | `Ragà, ho lascato il ** foglio** a Marta. Non è quello definitivo.` Fix the typo and bold. Keep `Ragà` and the negation. | Correct text and register. Added an unnecessary notice in English. |
| O3, new | Four paragraphs alternating between groceries and the workshop. Keep bread, Andrea calling the mechanic, milk only if discounted, and `Officina: forse giovedì, da confermare.` | Grouped the points, then repeated some of them and added commentary in English. |
| C6, long | `Riga N: ho incontratto Giulia, il preventivo non è approvato.` for 24 numbered paragraphs. Fix all 24 typos and keep all 24 negations. | Corrected 0 of 24 typos and kept 24 of 24 negations. |

The first round dropped emphasis in C3 and added a new claim in C2. The tuned prompt improved both repairs. The remaining problems include invalid source quotes, unnecessary notices, and duplicated or missed source points. A response marked `ok` at the protocol level remains a proposal subject to the editor's validation rules.

The organization fixtures used these selections.

**O1**

```markdown
Preventivo: Luca manda i prezzi.

Sale: chiedere la capienza a Giulia.

Preventivo: massimale 900 euro, IVA esclusa.

Sale: forse martedì, non confermato.
```

**O2**

```markdown
- Verificare il budget con Marta
- Cercare la ricevuta del treno
- Budget: non superare 180 euro
- Treno: orario ancora da decidere
```

**O3**

```markdown
Spesa: prendere il pane.

Officina: Andrea chiama il meccanico.

Spesa: latte solo se in offerta.

Officina: forse giovedì, da confermare.
```

The small fixtures took 1.26–4.05 seconds in the tuned run. The 1,632-byte encoded C6 request took 9.54 seconds. Peak helper RSS across these requests was about 20.1–21.4 MiB. C6's output failed the quality check, so its size gives no evidence for a supported selection limit.

`NSSpellChecker` was called with Italian (`it`) on the same five short correction inputs. Its spelling pass detected `incontratto`, `Riunone`, and the deliberately colloquial `Ragà`. It missed `lascato` and gave no grammar finding for `non e stato`. Delimiter repair was scored separately. The model detected more annotated edits on this sample. Its invalid notices and uncorrected long passage still prevent a release decision.

## Verification and decision

- All 28 workspace tests passed, including client protocol, deadline, and cancellation cases. Strict workspace Clippy, formatting, and the existing native editor smoke suite passed. The smoke run used the signed debug bundle and a temporary notes directory.
- The live client stopped a dense synthetic input at 30.01 seconds, rejected JSON above the request ceiling before launch, and decoded a fresh helper response afterward. The helper also returned `input_too_long` for an oversized frame sent directly. Framework context overflow and the preflight token boundary remain to be reproduced on the target models.
- The Rust client decoded live results from the signed helper for both actions with its own network access and the helper's blocked. A compatibility run on macOS 26.4 and a full-device offline run remain pending.
- Keep the 30-second deadline and token reserves provisional.

## Evaluation with source IDs

The helper now rebuilds organized notes from original units. The model proposes an order and grouping of IDs. The helper requires each ID once. It copies the source of a uniquely anchored uncertainty notice and supplies a fixed Italian label. Correction passes numbered prose fragments to the model and uses their original source ranges to apply replacements. The helper owns Markdown repairs and rejects changes to numeric expressions, recognized Italian negations, Markdown markers, and the number of words adjacent to a marker. The request and response JSON exchanged with Rust has the same shape.

| Fixture | Result in the final round |
| --- | --- |
| C1 | Fixed `incontratto` and preserved the negation. |
| C2 | Closed the bold marker. The proposal left `Riunone` uncorrected. |
| C3 | Repaired the stray space. The rest of the sentence matched the source. |
| C4 | Corrected `non e` to `non è` and preserved `-5` and the negation. An earlier variant tried to remove the negation. |
| C5 | Rejected the model response. The model expanded a fragment into the surrounding sentence. |
| C6 | Rejected the model response for the 24-paragraph passage. |
| C7 | `Regà, ho pagatto 28 euro. Non superare il ** tetto**.` An intermediate response repeated `tetto`. The final helper rejected the replacement under the word-count rule for fragments next to Markdown. |
| O1, O2, O3 | Grouped all points by topic, retained every original unit once, and quoted uncertainty from the source. |
| O4 | Grouped `Treno: prezzo 19 euro.` and `Treno: forse venerdì, da verificare.` apart from `Casa: chiamare Leo.` and `Casa: chiavi già consegnate.` The notice quoted the original train passage. |
| O5 | Reordered `- Da fare: inviare email`, `- Treno: binario 3`, `- Da fare: inviare email`, and `- Treno: orario da confermare`. It preserved both copies of the email entry. |

Both rounds used the same M4 and AFM 3 Core Advanced. For the final round, network access was denied to the helper process. The five organization fixtures took 1.10–1.14 seconds. The six short correction fixtures took 0.82–1.99 seconds, including rejected results. The long correction fixture returned its rejection after 10.76 seconds.

Swift self-checks exercised source ranges, unknown and duplicate IDs, retention of duplicate source entries, preservation of negations and quantities, and blocked replacements next to formatting. Build with `-D NOTE_REVISION_SELF_TEST` to run the local transformation checks without a model request. All 28 Rust workspace tests passed. The updated debug bundle and helper passed strict signature verification.

**Checkpoint.** The five organization fixtures pass. Correction passes C1, C3, and C4. C2 and the more complex correction cases remain open quality issues. Model-generated uncertainty notices are currently limited to organization.

## Editor verification

Both actions are integrated at the author's request. The editor validates complete Markdown blocks in the note body and checks the proposed replacement in the context of the full note before offering **Applica**. Organization uses source ranges from the editor's Markdown parse.

- On macOS 27.2.1, both commands produced an applicable preview in the signed debug bundle. The editor persisted the expected revision in a temporary archive for each action. The first Undo reverted the revision and the second reverted the preceding manual edit. Both Redo steps restored the edits in order.
- All 32 Rust workspace tests, strict Clippy, formatting, and the native smoke tests passed. The live model smoke tests denied network access to the app and its helper. The macOS model service retained its normal network configuration.
- Full-device offline operation and macOS 26.4 compatibility remain unverified.
