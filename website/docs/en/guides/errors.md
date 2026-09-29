---
title: Errors
description: Troubleshoot Aster API requests by HTTP status and error message.
---

# Errors

| HTTP status | Meaning | What to do |
| --- | --- | --- |
| `400` | Invalid field or parameter combination | Compare the request with the selected protocol and model guide. |
| `401` | Missing or invalid API key | Check the authentication header and key status. |
| `403` | The member cannot use the model | Confirm the grant in the member model page or contact an administrator. |
| `404` | Endpoint or public model ID not found | Check the base URL, path, and model ID. |
| `429` | Quota exhausted or rate too high | Review quota and retry with lower concurrency. |
| `502` / `503` | Provider temporarily unavailable | Keep the request ID and retry with backoff. |

The error body identifies the failed field or authorization reason. Do not diagnose from the status code alone.
