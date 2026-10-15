# Local Windows Thai editing update

This patch fixes existing-text edits that previously failed with
`the BIZ UDPGothic fallback font has no glyph for U+0E0A` (ช).
When a source subset cannot shape a Thai run, the editor uses a complete static Thai
face for the whole run and embeds its glyph mappings and positioning. It keeps a usable
embedded source face and honours explicitly selected document resources. The inline
preview does not apply automatic letter-spacing calibration to combining marks.

Sarabun Regular and Bold come unmodified from
[google/fonts at 88058c7](https://github.com/google/fonts/tree/88058c714073e72bb1b32baffc2d4ce9bc73944c/ofl/sarabun),
by the Sarabun Project Authors (cadsondemak), under OFL-1.1. Font binaries stay in the
external craft-fonts checkout; the preparation script verifies SHA-256 and records each
font and its licence there. The installer and portable package include those notices.

With Rust/MSVC, the Windows SDK and WiX 5.0.2 available, run from the repository root:

```powershell
./packaging/windows/prepare-thai-fonts.ps1 -FontRoot ../craft-fonts
$env:PDFCRAFT_VERSION = '0.8.0-thai.2'
$env:PDFCRAFT_BUILD_SHA = 'local-thai-fix'
./packaging/windows/package.ps1 -Arch x64
```

The MSI's numeric version remains 0.8.0. Its existing upgrade code and
`AllowSameVersionUpgrades` allow it to replace the installed 0.8.0 build. No automatic
installation is performed. Close PdfCraft and run the MSI to install the update.
The build is unsigned unless signing credentials are configured.

The private fork omits the upstream ArtCraft trademark artwork from its interface,
source tree and documentation. Upstream copyright, licence notices and plain-text
attribution remain intact.

Validation covers the reproduced failure, repeated edits and saves/reopens, real bold
fallback selection, keyboard entry/apply/undo/redo and an eleven-step CLI smoke test.
Thai word wrapping still uses spaces. Rotated text and contextual text across style/font
boundaries remain partial. The original user PDF was not supplied; validation uses
synthetic documents only.
