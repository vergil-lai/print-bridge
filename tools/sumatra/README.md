# SumatraPDF for Windows Printing

Windows V1 uses SumatraPDF for silent PDF submission.

This repository pins the official SumatraPDF 3.6.1 64-bit portable executable:

- Source: `https://www.sumatrapdfreader.org/dl/rel/3.6.1/SumatraPDF-3.6.1-64.zip`
- ZIP SHA-256: `98b33a518d42986856d225064b0cd2d3643ecf78cbf84ab873d26cc51877a544`
- EXE path: `src-tauri/resources/windows/SumatraPDF.exe`
- EXE SHA-256: `719f689b34f47be8ca105ce8484948474dafde0e106bab599e4a89326070c3d0`

`tauri.conf.json` bundles this file as the resource `SumatraPDF.exe`, and `src-tauri/src/lib.rs` resolves it with Tauri's resource resolver before constructing the Windows print backend.

Before release:

- Confirm SumatraPDF license compatibility. The upstream repository describes SumatraPDF as `(A)GPLv3` with some BSD-licensed code.
- Keep the exact version and hashes above in sync with the bundled executable.
- Confirm the Windows bundling/installer flow includes the resource and that `windows.rs` can resolve it at runtime.
- Verify command-line printing with a real or virtual Windows printer.
- Run antivirus/security scan on the bundled executable.
