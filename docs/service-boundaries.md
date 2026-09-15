# Service boundaries and customer responsibilities

[简体中文](zh-CN/service-boundaries.md) · [Documentation](README.md)

Aster Team is software for privately operating and governing access to supported AI services. It connects customer-authorized accounts to customer-operated infrastructure; it is not a seller or provider of those third-party services.

## What Aster provides

- A privately deployed gateway and control plane for supported model APIs.
- Connections to third-party accounts or subscriptions that the customer is authorized to use.
- Independent member identities and Aster API keys.
- Model access policy, quota allocation, usage accounting, and audit records.
- Customer-operated Runners for executing supported model requests through approved network destinations.
- OpenAI- and Anthropic-compatible interfaces plus documented client integrations.

## What Aster does not provide

- Third-party accounts, subscriptions, API credits, model entitlements, or account registration.
- A general-purpose VPN, Internet access service, network egress service, or geographic-access service.
- A mechanism for bypassing authentication, geographic restrictions, organizational policy, provider controls, rate limits, or terms of service.
- A guarantee that a particular provider, account, model, feature, or region will remain available.
- Legal, regulatory, tax, or contractual authorization to use a third-party service.

## Accounts, subscriptions, and charges

The **Subscriptions & accounts** area connects accounts already controlled by the customer. Aster does not create, resell, transfer, or bundle those accounts. The customer is responsible for account ownership or authorization, subscription status, upstream charges, and compliance with the provider's terms.

An Aster license controls Aster product entitlements. It does not purchase or expand upstream model access and does not replace any subscription, account, or usage charge required by a provider.

## Network responsibility

Aster routes supported model API traffic through customer-operated Control and Runner components. The customer must provide lawful and permitted connectivity from each Runner to the configured provider endpoints, including DNS, TLS trust, firewall rules, and any organization-approved egress configuration.

A Runner is an application-specific execution component, not a general network relay. Aster does not supply a VPN, general-purpose proxy, cross-region connection, or restriction-circumvention service. If a provider is not legally or technically available from the customer's environment, deploying Aster does not make that provider available.

## Availability and compatibility

Actual behavior depends on four independent layers:

1. Aster license entitlements.
2. Administrator policy and enabled models.
3. Customer account or subscription entitlements.
4. Provider availability, compatibility, and network reachability.

An integration documented by Aster means that the listed protocol or client flow is supported by Aster. It does not imply endorsement by the third-party provider or guarantee support for every upstream feature.

## Customer checklist

Before connecting an account or sharing an endpoint with members, confirm that:

- the organization is authorized to use the account and service;
- the selected deployment region and network path comply with applicable requirements;
- upstream charges and limits are understood;
- only customer-controlled Runner hosts can inspect request content;
- member keys, logs, backups, and account credentials are handled as sensitive data.
