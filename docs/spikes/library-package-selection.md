# Direct macOS library package selection

Evidence date: 2026-10-18. This report covers one tested platform: Google
Chrome 155.0.8059.82 on macOS 27.0.1 (build 26A434).

## Question

Can the browser receive an unchanged macOS Lytro library package after the
operator selects that package directly, without asking them to rename it,
select its parent directory or create a ZIP first?

## Method

The experiment used an ephemeral Chrome Guest profile and a probe served from
`http://127.0.0.1:18765`. Chrome reported that origin as a secure context and
exposed `showDirectoryPicker`. The probe offered three independent controls:

- `<input type="file" webkitdirectory>`;
- `<input type="file" accept=".lytrolibrary">`;
- `showDirectoryPicker()`.

The native chooser was operated directly. No DevTools Protocol file injection
or DOM assignment was counted as selection evidence.

An independently generated directory contained a 42-byte text file and a
nested 21-byte binary file. `SetFile -a B` set the directory's Finder bundle
bit without registering or changing any system file association.
`NSWorkspace.isFilePackageAtPath` then returned true for it and false for an
ordinary control directory. The source payload SHA-256 values were:

```text
70cf00fea36e4459db9b65e1b2fedf3cf25614d0a2834482429da47e4ddf10b0
1457c648d708a4b69a38dc85f1c5d137367e46975eddea9cd21543cacb2c2b06
```

The package was displayed by the chooser as a 63-byte “Lytro Desktop Library”
with the registered package icon. No private library, parent directory,
Launch Services association or running Lytronaut instance was read or changed.
The ordinary directory and a separately created ZIP were controls.

The small-file and platform checks were run with:

```sh
sw_vers
/Applications/Google\ Chrome.app/Contents/MacOS/Google\ Chrome --version
SetFile -a B <synthetic-package>
find <synthetic-package> -type f -print0 | sort -z | xargs -0 shasum -a 256
```

## Evidence

| Picker | Native package behavior | Exposed browser value |
|---|---|---|
| `webkitdirectory` | Package was visibly disabled and clicking it did not change the selected ordinary directory. | No package files. The ordinary-directory control produced two `File` values with root-relative paths and byte hashes matching its source files. |
| `showDirectoryPicker()` | Package was visibly disabled and clicking it left the parent directory selected. | No package handle. Cancelling rejected with `AbortError`; no files were exposed. |
| Normal file input accepting `.lytrolibrary` | Package was selectable and the native Open button was enabled. | One `File`: package basename plus `.zip`, MIME type `application/zip`, 729 bytes, and empty `webkitRelativePath`. |

Cancelling the normal file input returned to the probe with “No file chosen”
and produced no change event, browser value or import job.

Chrome produced the same ZIP SHA-256 on two selections:

```text
a04150890b4043e1e0decae07b346b257274b0565854ba151a3c322e8c239dbe
```

The generated ZIP used a single package-name root. It contained the two source
files at their original relative paths plus explicit root and nested-directory
entries. All four entries used method 8 (raw DEFLATE), UTF-8 names, zero ZIP
comments, zero central-directory extra fields and zero external attributes.
Each file entry had its expected expanded size. Each directory entry had zero
expanded bytes but a two-byte compressed DEFLATE stream. Decompressing the two
file entries reproduced the source sizes and both SHA-256 values above.
Selection therefore changes the transport envelope but not the file bytes or
hierarchy.

The exact generated ZIP does **not** pass the current Lytronaut intake unchanged.
Client admission accepts the two declared files and 63 expanded bytes, but the
native preflight source rejects any directory entry whose compressed size is
nonzero. A disposable isolated service also rejected the captured 729-byte ZIP:
the upload returned HTTP 400 `invalid_input`, and the job entered `failed` in
the `failed` phase. That binary had SHA-256
`57fe18ce3fab197590da00f700a53a7a8ecf8866a09040e62bf77f4324b055c9`
and was built before this report's commit, so the HTTP response is supporting
runtime evidence rather than a source-bound diagnosis of the exact rejection
branch. The source independently proves that Chrome's two-byte compressed
directory entries violate the current directory-size condition. Other framing
may also contribute to the runtime rejection and must be covered by the future
end-to-end regression.

The behavior agrees with current Chromium source. Its macOS bridge creates an
`NSOpenPanel`; all folder picker variants set `canChooseFiles = NO` and
`canChooseDirectories = YES`, while file selection uses the inverse. The same
bridge specifically permits an actual package through the extension-filtered
file path and rejects ordinary directories that happen to carry the accepted
suffix. See Chromium's current
[Chromium's macOS chooser bridge](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/components/remote_cocoa/app_shim/select_file_dialog_bridge.mm).
Apple documents that a default open panel treats file packages as files rather
than directories in [Using the Open and Save Panels](https://developer.apple.com/library/archive/documentation/FileManagement/Conceptual/FileSystemProgrammingGuide/UsingtheOpenandSavePanels/UsingtheOpenandSavePanels.html).
The File System Access draft defines `showDirectoryPicker()` as a secure-context
API returning a directory handle; it does not provide a package-specific mode:
[File System Access](https://wicg.github.io/file-system-access/).

## Verdict: GO

A browser-only direct package selection path is feasible on the tested
Chrome/macOS combination. The viable boundary is a normal file input filtered
to `.lytrolibrary`, followed by Chrome's generated ZIP and a narrowly extended
bounded ZIP intake. Neither directory picker can select a recognized package on
this platform. This is a GO for the selection approach and a NO-GO for routing
Chrome's output through the unchanged intake.

The verdict is deliberately platform-scoped. Chrome's package-to-ZIP behavior
is browser and operating-system integration, not a cross-browser web guarantee.
Safari, Firefox, other Chrome versions and non-macOS clients were not tested.
The normal file input does not require `showDirectoryPicker` or its secure
context API, although Lytronaut's deployed HTTPS boundary remains required for
the application as a whole.

## Recommendation

Replan a small implementation that adds a clearly labelled macOS library
package control backed by a normal, single-file input accepting
`.lytrolibrary`. When Chrome returns an `application/zip` file whose displayed
name has the package-plus-ZIP form, pass that `File` through the existing ZIP
admission and upload path after adding narrowly bounded support for Chrome's
valid empty DEFLATE directory entries. Verify that each accepted directory
payload inflates to empty and retains valid CRC and framing. Keep all current
central-directory, expanded-byte, file-count, per-file, path, overlap and server
preflight limits. Do not enumerate the source package, request write permission
or retain a local path.

Treat an empty selection as cancellation with no job creation. Preserve the
existing ordinary-directory and explicit-ZIP controls and their behavior. The
new label and status text must remain keyboard accessible and explain the
tested Chrome/macOS scope without presenting directory-picker failure as a
library error.

Acceptance for that future change must include a human-operated native
Chrome/macOS chooser check using an independently generated recognized package.
It must verify the generated ZIP's root, relative paths and payload hashes, plus
ordinary-folder, explicit-ZIP and cancellation regressions. Automated file
injection may cover upload logic but cannot replace the native chooser check.
Any implementation still requires its own reviewed plan and operator kickoff.
