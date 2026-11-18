# Security

## Official downloads

The only official Sparky PDF downloads are the files on this repository's
[Releases page](https://github.com/roberthendriksen/SparkyPDF/releases).
GitHub builds them from the source in this repository, using the workflow in
`.github/workflows/release.yml`. Copies offered anywhere else do not come from
this project.

Every release includes `SHA256SUMS.txt`. To check a download, run this in
PowerShell and compare the result with the line for that file:

```
Get-FileHash .\SparkyPDF-Setup.exe -Algorithm SHA256
```

Releases from 1.0.1 on also carry a build attestation: a record, signed by
GitHub, that the file was built by this repository's workflow. With the
[GitHub CLI](https://cli.github.com) installed:

```
gh attestation verify .\SparkyPDF-Setup.exe --repo roberthendriksen/SparkyPDF
```

The files are not code-signed, so Windows shows the publisher as unknown.

## Reporting a security problem

Please report it privately rather than in a public issue: open the
**Security** tab of this repository and choose **Report a vulnerability**.

In scope is everything in this repository:

- the file check and page recovery (`src/pdfcheck.rs`), which read untrusted files
- the rules for what the window may load, and the Windows registration (`src/main.rs`)
- the installer and the release workflow

Sparky PDF does not draw PDFs itself. Pages are rendered by Microsoft's
WebView2 Runtime, which Windows keeps up to date. A problem in how a PDF is
parsed or drawn should go to the
[Microsoft Security Response Center](https://msrc.microsoft.com/report).

Only the latest release is supported.
