# Bug remediation plan

Audit date: 2026-09-08. Reviewed baseline: `7dffedd5a23c3a52101a10bc4d27169d314f27cf`, plus the selection fix for issue #86.

## Delivery and scope

All 11 audit findings are published: eight new GitHub issues and three reopened reports with focused evidence. Every issue body or reopening comment was read back and checked against its prepared text. The original titles and bodies of reopened issues were preserved; their historical priority prefixes can differ from the priority of the specific remaining defect below.

All 11 findings below have implemented fixes and regression coverage. The audit used source traces, Windows fixtures, real UI buffer rendering, and workflow YAML validation. Process-action reproductions inspected target state without terminating user processes.

**User decision:** correct the command-line and environment feature claims; do not add retrieval APIs or privilege requirements for those features.

## Issue inventory

| Priority of current finding | Issue | Finding | Implementation status |
| --- | --- | --- | --- |
| P1 | [#87](https://github.com/faratech/htop-win/issues/87) | Tagged kill confirmation can switch to an untagged process | Implemented and verified |
| P1 | [#88](https://github.com/faratech/htop-win/issues/88) | Reused parent PIDs attach unrelated processes to branch actions | Implemented and verified |
| P1 | [#89](https://github.com/faratech/htop-win/issues/89) | Release workflow is invalid YAML due to unindented heredoc | Implemented and verified |
| P2 | [#10](https://github.com/faratech/htop-win/issues/10) | Header cap still leaves zero visible process rows | Implemented and verified |
| P2 | [#90](https://github.com/faratech/htop-win/issues/90) | Resizing while paused can leave the selected process off-screen | Implemented and verified |
| P2 | [#91](https://github.com/faratech/htop-win/issues/91) | Installed-version detection always returns None | Implemented and verified |
| P2 | [#92](https://github.com/faratech/htop-win/issues/92) | VIRT reports private commit instead of virtual address space | Implemented and verified |
| P2 | [#93](https://github.com/faratech/htop-win/issues/93) | Correct unsupported command-line and environment feature claims | Implemented and verified |
| P3 | [#72](https://github.com/faratech/htop-win/issues/72) | Suppressed metadata passes clear their own negative-cache timestamp | Implemented and verified |
| P3 | [#94](https://github.com/faratech/htop-win/issues/94) | Error dialogs hide the dismissal hint and truncate diagnostics | Implemented and verified |
| P3 | [#15](https://github.com/faratech/htop-win/issues/15) | Skin-tone emoji consume excess cells and truncate text early | Implemented and verified |

## Existing selection fix: #86

[Issue #86](https://github.com/faratech/htop-win/issues/86) has an implementation in `src/app.rs`, a rendering/input-order comment in `src/main.rs`, and documentation in `README.md`. Normal refreshes keep selection and scroll stationary; explicit `F` mode follows process identity. Action dialogs capture identity when invoked and retain PID-reuse verification.

The previous verification passed all 144 Windows tests, Clippy with warnings denied, and an x64 release build. Those were the baseline results for #86. The combined verification below includes the additional fixes and preserves the stationary-selection and explicit-follow regressions. Do not create another issue for it or close it before integration.

## Remediation order and acceptance criteria

The following sections record the implemented behavior and verified acceptance criteria. Integrate the work in this order and keep review grouped by subsystem. Checked acceptance items describe corrected behavior, not tests asserting the old defect.

### 1. Process safety

#### [#87: Tagged kill confirmation can switch to an untagged process](https://github.com/faratech/htop-win/issues/87)

A termination dialog opened for tagged process A can switch to untagged selected process B after A exits. Confirming then targets B; PID identity verification does not prevent this because B has a valid captured identity.

**Evidence:** `src/app.rs::enter_kill_mode` captures the selected process rather than an immutable batch. `update_displayed_processes` prunes exited tags. Keyboard and mouse kill confirmation in `src/input.rs` choose batch versus single termination using the current tag set.

**Reproduction:**

1. Tag A, then leave untagged B selected.
2. Press F9; the dialog asks to terminate tagged processes.
3. Let A exit and allow a refresh.
4. The dialog switches to the single-process target B.

A Windows fixture exercised these exact state transitions and rendered both dialog states. No termination was executed during validation.

**Implementation:** Capture an immutable termination request when the action opens: either one target or a batch containing identities and display names. Render and confirm solely from that request. Exited batch targets are unavailable; never fall back to selection. Preserve identity verification and readonly enforcement.

**Acceptance:**

- [x] All tagged targets exiting cannot dispatch termination to an untagged process.
- [x] Partial exits, PID reuse, and filtered tags retain the original requested identities.
- [x] Keyboard and mouse confirmation dispatch the same captured request.
- [x] Use a test action sink to assert identities without killing real processes.

#### [#88: Reused parent PIDs attach unrelated processes to branch actions](https://github.com/faratech/htop-win/issues/88)

A new process that reuses an exited parent PID adopts older unrelated surviving children in the tree and branch tagging. A later batch termination can therefore include unintended processes.

**Evidence:** `src/app.rs::build_tree`, `branch_identities`, and `collapse_to_parent` infer relationships from PPID equality without validating the available creation timestamps.

**Reproduction:**

Create a fixture with current parent PID P created at time 200 and child C created at time 100 with PPID P. Tree rendering nests C under P and tagging P’s branch tags C. A Windows harness reproduced both behaviors without invoking process actions.

**Implementation:** Build one validated parent map shared by tree rendering, branch tagging, and parent navigation. Accept an edge only when both creation times are nonzero and parent creation time is no later than child creation time. Treat rejected children as roots. Keep cycle protection.

**Acceptance:**

- [x] An older child cannot be attached to or tagged through a newer owner of its parent PID.
- [x] Valid parent/child/grandchild chains and equal nonzero timestamps work.
- [x] Missing parents, unknown timestamps, and cycles terminate safely and consistently across tree operations.

### 2. Release and viewport correctness

#### [#89: Release workflow is invalid YAML due to unindented heredoc](https://github.com/faratech/htop-win/issues/89)

The release workflow cannot be parsed, so release jobs never start.

**Evidence:** `.github/workflows/release.yml:214-224`: release-note heredoc content and its EOF marker are at column zero, outside the indented `run: |` scalar.

**Reproduction:**

Parsing the file with PyYAML fails at line 220: "found character ` that cannot start any token". GitHub also reports a failed workflow run at the reviewed commit: https://github.com/faratech/htop-win/actions/runs/34014522476

**Implementation:** Indent the complete heredoc inside the YAML run block, so YAML removes the common indentation and Bash receives a valid EOF delimiter. Add actionlint workflow validation to CI. Preserve the existing release tag, package-version, and exact-SHA checks.

**Acceptance:**

- [x] All workflow files pass YAML parsing and actionlint.
- [x] The extracted release-note shell script passes bash -n and emits the verification section using a local fixture.
- [x] Tag/package mismatch and missing-tag validation remain intact.
- [x] No release needs to be published to validate this fix.

#### [#10: Header cap still leaves zero visible process rows](https://github.com/faratech/htop-win/issues/10)

The header-starvation part of this issue remains reproducible. The current cap reserves a column-heading row but no process data row.

**Evidence:** `src/ui/mod.rs::draw`, around line 39, caps the header with terminal height minus tab bar, two footer rows, and one table-heading row.

**Reproduction:**

A real UI-rendering fixture with 64 CPUs, three loaded processes, and an 80x24 terminal produces header 0..21, process region 22..22, and visible_height=0.

**Implementation:** Reserve three process data rows when space permits, separately accounting for the table heading, tabs, and footer. Shrink the header first on smaller terminals. Keep all computed regions inside terminal bounds.

**Acceptance:**

- [x] 64- and 128-core fixtures at 80x24 render actual process content.
- [x] One-tab and multiple-tab layouts preserve process rows.
- [x] Short terminals prioritize process content over header meters whenever space permits.
- [x] Zero-height and tiny-terminal layouts stay within bounds.

[Focused reopening evidence](https://github.com/faratech/htop-win/issues/10#issuecomment-5579079712).

#### [#90: Resizing while paused can leave the selected process off-screen](https://github.com/faratech/htop-win/issues/90)

Resizing the terminal updates viewport height without reconciling scrolling. While paused, selection can remain hidden indefinitely, and F9 still targets the off-screen process.

**Evidence:** `src/main.rs::run_app` only requests redraw for Resize. `src/ui/mod.rs::draw` assigns visible_height before rendering but does not normalize selected_index and scroll_offset.

**Reproduction:**

A real UI fixture with 40 processes, paused=true, selected_index=15, scroll_offset=0 is visible at 120x24. Resize to 120x10: visible_height becomes 7, but selection remains 15 with offset 0 across repeated redraws. F9 captures off-screen PID 16.

**Implementation:** Normalize selection and scroll immediately after calculating viewport height and before rendering process rows. Preserve the selected index when valid and move only the viewport as needed. Apply to terminal resize and header visibility changes; retain #86’s stationary selection during ordinary sorting refreshes.

**Acceptance:**

- [x] Paused shrink/grow keeps selection visible whenever data rows are available.
- [x] Header toggles normalize scrolling immediately.
- [x] Empty lists and transitions through zero-height viewports remain bounded.
- [x] A subsequent action targets the visibly selected row.
- [x] Existing #86 dynamic-sort and explicit-follow tests still pass.

### 3. Metadata and accurate feature claims

#### [#91: Installed-version detection always returns None](https://github.com/faratech/htop-win/issues/91)

Installed-version detection fails for a valid versioned executable. Installation checks cannot recognize an already-current installed copy, and update success messages report an unknown version.

**Evidence:** `src/installer.rs::read_pe_file_version`, around lines 72-81, passes `&mut fixed.cast()` to VerQueryValueW. The API writes a temporary pointer value, while the original fixed pointer stays null and triggers the None return.

**Reproduction:**

A Windows fixture copied the built executable into an isolated LOCALAPPDATA installation layout. get_installed_version returned None. Independently, Windows FileVersionInfo read the same executable as 0.2.8. No actual installation was changed.

**Implementation:** Pass mutable output-pointer storage to VerQueryValueW, then validate the returned pointer, length, and version-record signature before reading the numeric version. Retain metadata-only inspection; do not execute the installed binary.

**Acceptance:**

- [x] A real versioned PE fixture returns its embedded three-part version.
- [x] Missing, malformed, or absent version resources return None safely.
- [x] Same-version installation checks recognize an installed copy.
- [x] The test uses an isolated fixture path and never executes the inspected binary.

#### [#92: VIRT reports private commit instead of virtual address space](https://github.com/faratech/htop-win/issues/92)

VIRT and the process-details Virtual Memory field report private commit, excluding reserved virtual address space despite their labels.

**Evidence:** `src/system/native.rs::SystemProcess::virtual_size` returns info.pagefile_usage. The raw structure already contains the separate virtual_size field.

**Reproduction:**

A Windows fixture successfully reserved 1,073,741,824 bytes using VirtualAlloc(MEM_RESERVE). The displayed VIRT value rose by only about 0.4 MiB of incidental commit rather than reflecting the reservation.

**Implementation:** Return the existing raw virtual_size field. Preserve VIRT’s existing name, persisted column key, and sorting interface. Leave resident/shared memory accounting unchanged.

**Acceptance:**

- [x] A fixture with different commit and virtual-size fields returns virtual size.
- [x] A successful 1 GiB reservation is reflected in VIRT.
- [x] VIRT sorting follows virtual address-space size.
- [x] Resident and shared memory regressions remain unchanged.

#### [#72: Suppressed metadata passes clear their own negative-cache timestamp](https://github.com/faratech/htop-win/issues/72)

The 15-second failure backoff does not survive a suppressed enrichment pass. Protected-process metadata is retried every other pass instead of waiting for expiry.

**Evidence:** `src/system/process.rs` suppresses need_* flags while query_failed_at is fresh. The suppressed pass then reports query_failed=false, and the cache update around lines 917-921 clears the timestamp.

**Reproduction:**

A Windows fixture seeded a fresh failure timestamp and requested unknown architecture metadata. After one suppressed pass, the timestamp changed from present to absent while architecture remained unknown; no successful query justified clearing it.

**Implementation:** Represent skipped, attempted-success, and attempted-failure outcomes separately. Preserve the original timestamp on skipped/suppressed passes, stamp failed attempts, clear after a successful retry, and reset on identity changes.

**Acceptance:**

- [x] Repeated suppressed passes preserve the original timestamp.
- [x] Expiry permits a retry.
- [x] Successful retries clear failure state; failed retries restart backoff.
- [x] PID reuse invalidates the previous identity’s failure state.

[Focused reopening evidence](https://github.com/faratech/htop-win/issues/72#issuecomment-5579080789).

#### [#93: Correct unsupported command-line and environment feature claims](https://github.com/faratech/htop-win/issues/93)

The application advertises a full command line and environment inspection, but command text contains only an executable path/name and the environment dialog does not query environment variables.

**Evidence:** `src/system/process.rs` assigns executable path to ProcessInfo.command in constructors and enrichment. `src/ui/dialogs.rs::draw_command_wrap` labels it Command Line; draw_environment always displays a privilege explanation instead of environment data. README and built-in help promise these capabilities.

**Reproduction:**

A Windows process launched with --audit-marker=expected-in-command retained that argument in its actual argument list, but ProcessInfo.command contained only its executable path. Source inspection confirms environment inspection is not implemented even for elevated or same-user processes.

**Implementation:** User-selected scope: correct the claims, do not implement retrieval. Rename w to "Wrap executable path" in help and documentation. Remove misleading or duplicate command-line sections from details and update field comments. Preserve e as a compatible key that explicitly says "Environment inspection is not implemented." Remove the claim that elevation would make it work. Preserve existing Command column/config keys.

**Acceptance:**

- [x] README, built-in help, dialog titles, and descriptions match the available data.
- [x] w presents executable path/name without claiming to include arguments.
- [x] e explicitly states that inspection is not implemented without suggesting elevation fixes it.
- [x] Existing saved column and sort keys continue to load.
- [x] No command-line/environment retrieval API or new privilege requirement is introduced.

### 4. Rendering polish

#### [#94: Error dialogs hide the dismissal hint and truncate diagnostics](https://github.com/faratech/htop-win/issues/94)

The error overlay always clips its dismissal hint and can hide the useful part of a long diagnostic.

**Evidence:** `src/ui/dialogs.rs::draw_error`, around lines 946-949, fixes total height at five rows (three interior rows) but always constructs at least four content rows: blank, error, blank, and dismissal hint.

**Reproduction:**

Actual UI buffer rendering at a roomy terminal shows border, blank, error, blank, border. The "Press any key to dismiss" line is absent. Wrapped errors exceed the fixed interior with no scroll path.

**Implementation:** Size the overlay from wrapped content, cap it to terminal bounds, and pin a visible hint. Store overflow scroll state and reset it for a new error. Navigation keys scroll; other keys dismiss, with global quit retaining precedence.

**Acceptance:**

- [x] Short errors show both the message and dismissal hint.
- [x] Long/multiline diagnostics and Windows paths are fully reachable on narrow terminals.
- [x] Scrolling resets for a new message.
- [x] Global quit works and ordinary dismissal does not also execute an underlying process action.

#### [#15: Skin-tone emoji consume excess cells and truncate text early](https://github.com/faratech/htop-win/issues/15)

A specific Unicode-width defect remains: skin-tone modifiers are split from their base emoji, so layout consumes excess cells and truncates text prematurely.

**Evidence:** `src/terminal.rs::TerminalSymbols::next`, around line 861, groups zero-width characters and ZWJ but not positive-width emoji modifiers.

**Reproduction:**

Actual Buffer rendering of 👍🏽A stores the base at column 0, modifier at 2, and A at 4. The project’s unicode-width reports total width 3, so A should start at column 2. Truncating to three cells loses content that should fit.

**Implementation:** Use unicode-segmentation extended grapheme clusters before the existing aggregate-width calculation and continuation-cell rendering. Preserve terminal-control sanitization and inline symbol storage. Apply the same boundaries to truncation.

**Acceptance:**

- [x] 👍🏽A positions A at column 2 and fits in three cells.
- [x] Modifier-plus-ZWJ, combining marks, and flags render and truncate as whole clusters.
- [x] Continuation cells and diff rendering remain correct when glyph widths change.
- [x] Existing control-character sanitization regressions pass.

[Focused reopening evidence](https://github.com/faratech/htop-win/issues/15#issuecomment-5579081311).

## Interfaces and compatibility

- Replace mutable selection-dependent kill dispatch with an immutable termination-request type shared by rendering, keyboard confirmation, and mouse confirmation. Capture identities and display names for the entire batch at action time. Keep readonly guards and verified process handles.
- Use one parent-relationship definition for tree rendering, branch tagging, and navigation. Require nonzero timestamps with parent creation no later than child creation; equal nonzero times remain valid. Reject unknown/impossible ancestry consistently and retain cycle protection.
- Keep CLI flags, persisted column names, and config schema compatible. The command/environment correction changes labels, help, and unsupported-feature messaging; it does not introduce data retrieval.
- Give overflowing errors explicit scroll state, reset for each new error. Navigation scrolls the error; non-navigation dismissal must not execute a command underneath it. Preserve global quit precedence.
- Use `unicode-segmentation` extended grapheme boundaries with the existing `unicode-width` calculations. Keep control sanitization and inline symbol storage. Validate dependency changes with the Windows target and release builds.

## Verification and closure

For each patch, run its focused regression tests, then the relevant Windows suite and Clippy. On this Linux/WSL host, plain native Cargo tests do not exercise this Windows-only crate. Bypass the host's stalled compiler-cache wrapper when needed:

```bash
RUSTC_WRAPPER= cargo test --target x86_64-pc-windows-gnu --all-targets
RUSTC_WRAPPER= cargo clippy --target x86_64-pc-windows-gnu --all-targets -- -D warnings
```

- Validate all workflow files with actionlint and YAML parsing. Test extracted shell syntax and release-note generation with local fixtures; do not publish a release as a syntax test.
- Build x64 and ARM64 Windows release artifacts with the canonical `build-cross.py` workflow before merging the complete remediation. The script copies successful builds to its configured Windows output directory.
- Keep process-action testing isolated: use a test action sink for dispatched identities, and temporary fixture paths for installed-version checks. Do not terminate real user processes or overwrite an actual installation during tests.
- Add links to fixing PRs and their validation results to the corresponding issues. Close each issue only after its acceptance criteria pass and its fix is merged. On broad reopened reports, distinguish the newly resolved residual defect from the historical fixes.
- Integrate and verify #86 with the viewport patches. Release publishing and version changes are follow-up work, not part of this issue-publication delivery.

## Local verification record — 2026-09-08

- The combined Windows suite passes **158 tests** across library, CLI, installed-version, process-metadata, and actual UI-rendering fixtures.
- Windows x64 Clippy passes for all targets with warnings denied.
- All workflow YAML and Bash syntax checks pass; actionlint 1.7.12 passes. An isolated release-source fixture accepts a matching tag and rejects both missing tags and tag/package-version mismatches. A release-note fixture emits both architecture verification commands.
- The canonical `RUSTC_WRAPPER= python3 build-cross.py build-all` builds Windows x64 and ARM64 release executables and copies them to `/mnt/c/code/htop-win-x64.exe` and `/mnt/c/code/htop-win-arm64.exe`. Both executables pass the `--version` launch check. The ARM64 toolchain reports an unused `-no-pie` linker-argument warning; build and launch succeed.
- The application version is 0.2.9. Local verification does not publish a release.

### Regression coverage

| Finding | Regression coverage |
| --- | --- |
| #87 | `tagged_confirmation_never_retargets_after_exits_or_pid_reuse`; `partial_batch_exits_and_readonly_preserve_captured_targets` — keyboard/mouse use a test termination sink; filtered tag names come from canonical process data. |
| #88 | `parent_edges_are_validated_for_tree_tags_and_navigation` plus existing deep-tree/cycle tests. |
| #89 | CI now runs actionlint; local YAML, Bash, tag-validation, and release-note fixtures passed. |
| #10 | `many_core_headers_reserve_real_process_rows` — 64/128 cores, one/multiple tabs, and tiny terminal heights. |
| #90 / #86 | `paused_resize_and_header_toggle_keep_selection_visible` plus stationary-sort and explicit-follow tests. |
| #91 | `tests/installed_version.rs` checks a real versioned application, a PE without the version resource, malformed data, and a missing file in an isolated installation layout. |
| #92 | Native accessor and sorting fixtures; `tests/process_metadata.rs` confirms a 1 GiB reservation affects VIRT without committing physical RAM. |
| #72 | `tests/process_metadata.rs` exercises repeated suppression, expiry/success, verified-handle failure, and identity changes. |
| #93 | `unsupported_views_describe_the_available_data`; README/help labels and persisted column keys reviewed. |
| #94 | `errors_pin_hint_and_scroll_all_diagnostics_without_dismissing` — long-lived errors remain readable, replacement resets scrolling, and dismissal does not execute F9. |
| #15 | Buffer placement, truncation, diff replay, input-window scrolling, and dialog wrapping tests preserve whole grapheme clusters; existing control-sanitization tests pass. |

No real user process was terminated, no actual installation was modified by the fixtures, and no release was published. GitHub issue closure remains contingent on integration as described above.
