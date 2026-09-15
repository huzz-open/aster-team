# Aster Team error codes and troubleshooting reference

[简体中文](../zh-CN/guides/aster-error-codes.md) · [Deployment guides](README.md) · [Troubleshooting](../troubleshooting.md)

Aster API and administration errors contain a stable string code, a stable five-digit number, and a user-safe message. Use the number to search this reference and the request ID to trace one execution.

```json
{
  "error": {
    "code": "RUNNER_NOT_READY",
    "message": "No compatible runner is currently ready.",
    "number": 33003
  }
}
```

The number is also returned in `X-Aster-Error-Number`. `X-Aster-Request-ID` identifies one request and is not an error code.

## Model API errors

| Number | Code | Meaning and next action |
| ---: | --- | --- |
| `31001` | `INVALID_API_KEY` | The key is missing, invalid, revoked, or belongs to a disabled member. Create or activate the correct member key. |
| `32001` | `INVALID_REQUEST` | A request field, query parameter, or model variant is invalid. Compare the request with the API reference. |
| `33001` | `MODEL_NOT_FOUND` | The model is unknown, disabled, or lacks an account mapping. Query `/v1/models` and review Admin model settings. |
| `33002` | `MODEL_ACCOUNT_NOT_READY` | The model exists, but no usable connected credential is ready. Review account health and refresh status. |
| `33003` | `RUNNER_NOT_READY` | No Runner satisfies health, protocol, heartbeat, and capacity checks. Test Runner connectivity. |
| `33004` | `RUNNER_RETRY_FORBIDDEN` | Upstream execution or output already began, so Aster will not silently replay the request on another Runner. Decide whether the operation is safe to retry. |
| `34001` | `INSUFFICIENT_QUOTA` | Available member quota is too low after in-flight reservations. Review quota and current requests. |
| `34005` | `DUPLICATE_REQUEST` | The accounting request ID was already reserved or settled. Generate a new operation ID when a new charge is intended. |
| `35001` | `INVALID_UPSTREAM_RESPONSE` | The Runner response could not be validated. Preserve the request ID and inspect both Control and Runner logs. |
| `35002` | `UPSTREAM_REQUEST_FAILED` | A Runner accepted the request, but provider transport or execution failed. Check the upstream status and trace. |
| `35003` | `UPSTREAM_RESPONSE_TOO_LARGE` | The response exceeded the Control processing limit. Reduce the requested output. |
| `35004` | `UPSTREAM_USAGE_INVALID` | Trusted usage was missing or invalid, so Aster refused to invent a settlement value. Trace the request. |

## License errors

| Number | Code | Meaning and next action |
| ---: | --- | --- |
| `51001` | `LICENSE_MISSING` | Install the bundled or issued signed license. |
| `51002` | `LICENSE_EXPIRED` | Import a renewal or switch to the free license after reducing resources to its limits. |
| `51003` | `LICENSE_INVALID` | The license structure, signature, time, or claims are invalid. Re-import the original issued file. |
| `51004` | `LICENSE_TIME_INVALID` | Correct the host time and review the trusted-time state. |
| `51005` | `LICENSE_MACHINE_MISMATCH` | Obtain a license issued for this installation identity. |
| `51006` | `LICENSE_VERSION_INVALID` | Upgrade Aster to the license's minimum supported version. |
| `51008` | `FEATURE_NOT_LICENSED` | Review the active feature set in **Product license**. |

## Installation and maintenance errors

| Number | Code | Meaning and next action |
| ---: | --- | --- |
| `43003` | `DELIVERY_RELEASE_INVALID` | Package signature, file tree, or layout validation failed. Download and verify the release again. |
| `43004` | `DELIVERY_MAINTENANCE_BUSY` | Another install, upgrade, restore, or maintenance task holds the lock. Inspect the active task instead of deleting lock state. |
| `43006` | `DELIVERY_UPGRADE_FAILED` | Upgrade failed and the recovery policy ran. Check final service health and diagnostics. |
| `43010` | `DELIVERY_PLATFORM_UNSUPPORTED` | The package or required host capabilities do not match this machine. Use a supported Release artifact. |
| `43012` | `DELIVERY_LICENSE_FAILED` | License request, import, or status validation failed. Run `license status` and preserve the original file. |
| `43013` | `DELIVERY_BACKUP_FAILED` | Backup preflight, creation, restore, or restore rollback failed. Check storage, permissions, identity, and diagnostics. |
| `43014` | `DELIVERY_RUNNER_FAILED` | Runner enrollment, configuration, or service management failed. Inspect Runner status and logs. |
| `43015` | `DELIVERY_DIAGNOSTIC_FAILED` | One or more installation, database, TLS, Runner, or service health checks failed. Run `doctor --verbose`. |

## Administration and runtime errors

| Number | Code | Meaning and next action |
| ---: | --- | --- |
| `12010` | `MEMBER_SEAT_LIMIT_REACHED` | Enabling another model-consuming member would exceed the license. Disable an unused member or update the license. |
| `13008` | `UPSTREAM_OAUTH_SESSION_INVALID` | The OAuth session, callback, state, or expiry is invalid. Start a new authorization flow. |
| `13009` | `UPSTREAM_OAUTH_EXCHANGE_FAILED` | The provider rejected the authorization exchange or returned an invalid identity. Check account authorization and Runner connectivity. |
| `13011` | `UPSTREAM_MODEL_SYNC_FAILED` | The provider model endpoint failed or returned no usable models. Check the existing account and Runner before retrying synchronization. |
| `13015` | `UPSTREAM_ACCOUNT_LIMIT_REACHED` | The logical account limit is reached. Delete an unused account or update the license; disabling it does not release capacity. |
| `14001` | `RUNNER_ENROLLMENT_INVALID` | The one-use enrollment token is missing, expired, or already used. Create a new token. |
| `14004` | `RUNNER_LIMIT_REACHED` | The Runner entity limit is reached. Delete an unused Runner or update the license. |
| `15011` | `MAINTENANCE_BUSY` | Another upgrade or cleanup task is queued or running. Monitor the existing task. |
| `22004` | `API_KEY_LIMIT_REACHED` | This member has reached the active-key limit. Revoke an old key before creating another. |
| `91001` | `BACKEND_STORAGE_UNAVAILABLE` | The service cannot safely access the business database. Check storage health and permissions; retry after recovery. |
| `91002` | `BACKEND_DATA_INTEGRITY_INVALID` | Installation-bound integrity validation failed. Stop write attempts and restore from a known-good backup or contact support. |
| `91003` | `BACKEND_INSTANCE_DRAINING` | The instance is draining and rejects new work with `503` and `Retry-After`. Wait for maintenance to finish. |
| `91004` | `BACKEND_REQUEST_DEADLINE_EXCEEDED` | The original processing deadline expired. Confirm the operation result before retrying a write. |

## Collect a trace

For a failed model request, copy `X-Aster-Request-ID` or the request ID from the JSON response, then run on Control:

```bash
sudo aster-team-cli trace
```

Provide the exact time and time zone, endpoint, model, five-digit number, string code, and redacted `status` and `doctor` output. Remove passwords, API keys, cookies, access tokens, refresh tokens, license files, private keys, prompt bodies, and response bodies before opening an issue.
