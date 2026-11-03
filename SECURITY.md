# Security Policy

## Supported Versions

The current development line is the supported version.

## Reporting a Vulnerability

Please do not report security vulnerabilities through public GitHub issues.
Send reports to [security@frontal.dev](mailto:security@frontal.dev).

Include the affected component or API, deployment context, reproduction steps,
impact assessment, and any proof of concept that can be shared safely. Do not
include production credentials or personal data.

We will acknowledge reports within 48 hours and coordinate disclosure with the
reporter.

## Security model

Sandbox sessions follow a capability-based model. Network access is denied by
default, host access is mediated by policy checks, and isolate sessions are not
treated as a security boundary without the worker and microVM layers.
