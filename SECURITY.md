# Security Policy

## Supported versions

QR Lab is pre-1.0. Security fixes are applied on the default branch
(`develop`). Older tags and compatibility facades are not separately patched.

## Reporting a vulnerability

Please **do not** open a public GitHub issue for security problems.

Report privately through one of:

1. [GitHub private vulnerability reporting](https://github.com/aukilabs/qr-lab/security/advisories/new)
2. Email [contact@aukilabs.com](mailto:contact@aukilabs.com) with the subject
   `QR Lab security`

Include enough detail to reproduce the issue: affected crate or binding, a
minimal input if possible, and the impact (crash, incorrect decode that could
be trusted, memory unsafety, and so on).

You should receive an acknowledgement within a few business days. We will
coordinate a fix and a public disclosure once a patch is ready.

## Scope notes

- The scanner is designed to take untrusted image buffers. Crashes, panics on
  malformed frames, and memory-safety issues in the Rust crates or C ABI are
  in scope.
- Incorrect QR payloads on pathological inputs are generally quality bugs, not
  security bugs, unless an application could reasonably treat the payload as
  authenticated.
- Please do not include secrets or production camera captures in reports unless
  they are required to reproduce the issue.
