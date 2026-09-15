# Member guide

[简体中文](zh-CN/member-guide.md) · [Documentation](README.md)

The Member UI is the self-service entry point for developers. It shows the platform endpoint, enabled models, quota and usage, API key management, and client-specific setup commands.

An Aster member API key grants access only through the policy configured by the administrator. It is not a third-party provider account, subscription, or credit. Availability still depends on the customer-connected account, Runner connectivity, and provider service. See [Service boundaries](service-boundaries.md).

## First sign-in

Sign in with the temporary password delivered by your administrator and change it immediately. Passwords and full API keys are not recoverable. If a key is lost, revoke it and create another one.

## Create an API key

Create a separate key for each device, development environment, or automation identity. Copy the value when it is shown and store it in the operating system credential store or a secrets manager—not in source control, screenshots, shell history, or issue reports.

The OpenAI-compatible base URL is:

```text
https://aster.example.com/v1
```

The Anthropic-compatible base URL omits `/v1`:

```text
https://aster.example.com
```

Use the exact public URL shown by your Member UI.

## Choose a model

Query `/v1/models` or use the model list on the integration page. It contains only models enabled by the administrator. Some clients can also use an Aster model variant for speed or reasoning effort; see [Model variants](api-reference.md#model-variants).

## Integrations

- Use the generated curl examples for protocol-level testing.
- Download `asterctl` from **API documentation → asterctl tool** to configure [Codex](codex.md) or [Claude Code](claude-code.md).
- Keep one API key per user even if multiple tools use it, or create separate keys when independent revocation is useful.

## Usage and errors

The Member UI shows granted, reserved, and consumed quota plus per-request usage records. When reporting a failure, provide the timestamp, endpoint, model, `X-Request-ID`, and Aster's five-digit error number after redacting content and credentials.
