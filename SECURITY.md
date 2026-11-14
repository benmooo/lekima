# Security

## Reporting

Email security concerns to the maintainer privately (see the GitHub profile)
rather than opening a public issue. Please include reproduction steps.

## Credentials

lekima stores the NetEase session cookie in
`<config>/session.cookies` with mode `0600`. It is never written to logs, argv
or the IPC stream. `lekima logout` removes it. The MCP server never reads the
file. If you believe a token leaked, run `lekima logout`, revoke the session in
the NetEase app, and log in again.

## Supported versions

The latest 0.1.x release receives security fixes.
