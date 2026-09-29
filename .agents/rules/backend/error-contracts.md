# Error contracts

- Preserve the original error type through storage and service layers. Do not stringify a typed failure before selecting its API error descriptor.
- Return a stable, cause-specific code and actionable safe message for distinguishable conditions such as connection failure, schema or decoding mismatch, query failure, local state failure, invalid input, conflict, and missing resources. Do not funnel unrelated errors into one generic storage code.
- Do not reintroduce `BACKEND_STORAGE_UNAVAILABLE` (`91001`) or replace one broad fallback with another for known failures. Classify typed database and runtime errors before building the response.
- Keep raw SQL, filesystem paths, credentials, and upstream response bodies in structured server logs; never return them in public API messages.
- Map persistence failures consistently for all endpoints, including admin, member, and gateway paths. A missing Runner must use the Runner readiness code only when routing actually requires a Runner.
- Add or update the central error catalog and generated contracts when introducing a public error. Verify both the response status and error number in focused tests.
