---
title: Image generation and editing API
description: Distinguish generation from editing, image count quota, input formats, output delivery, and model limits in Aster.
---

# Image generation and editing

Images are separate operations. Use `POST /v1/images/generations` to generate and `POST /v1/images/edits` to edit. Select a granted public **image model**; text access does not automatically grant image generation.

## Generation

Set `ASTER_BASE_URL` to the API URL shown in the [quickstart](/en/start/quickstart), then replace `YOUR_IMAGE_MODEL` with a granted image model ID.

<InstanceCurlExample language="en" kind="generate" />

Generic template:

```bash
curl --fail-with-body "$ASTER_BASE_URL/v1/images/generations" \
  -H "Authorization: Bearer $ASTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"YOUR_IMAGE_MODEL","prompt":"A cat on a rainy city street at night","n":1}'
```

```json
{"model":"YOUR_IMAGE_MODEL","prompt":"A cat on a rainy city street at night","n":1}
```

`n` is the requested image count; limits vary by model. Aster checks `size`, `quality`, `output_format`, and transparent background by model and combination. When a provider returns a short-lived URL, Aster must fetch and validate the asset before returning unified `b64_json`. A delivery failure must not resubmit generation.

## Editing

Upload a local source image as `multipart/form-data`. Do not set `Content-Type` manually; cURL adds the required boundary. Use a model that supports editing.

<InstanceCurlExample language="en" kind="edit" />

Generic template:

```bash
curl --fail-with-body "$ASTER_BASE_URL/v1/images/edits" \
  -H "Authorization: Bearer $ASTER_API_KEY" \
  -F "model=YOUR_EDIT_MODEL" \
  -F "prompt=Replace the background with a night scene" \
  -F "image=@source.png"
```

Editing needs a source image. An optional mask has model-specific format and size limits. A provider’s generation endpoint does not imply editing support. Check your installation’s exposed operations before calling.

Image count is accounted for separately from token quota. Missing upstream token usage is not converted into estimated tokens.

## Image model matrix

Select an image model and operation to inspect field support and value mappings. These are the signed plugin rules; actual availability also depends on the installation and member permissions.

<ImageFieldMatrix language="en" />
