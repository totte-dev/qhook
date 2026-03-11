# CallbackApi

All URIs are relative to *http://localhost:8888*

| Method | HTTP request | Description |
|------------- | ------------- | -------------|
| [**callback**](CallbackApi.md#callback) | **POST** /callback/{token} | Resume a waiting workflow step |



## callback

> Callback200Response callback(token, requestBody)

Resume a waiting workflow step

Resumes a workflow step that is waiting for a callback. The token is a cryptographic identifier generated when the callback step was created.  The request body becomes the step output, available to subsequent steps via response chaining.  Returns a uniform 404 for all failure cases (invalid, expired, or already-used tokens) to prevent token enumeration. 

### Example

```ts
import {
  Configuration,
  CallbackApi,
} from 'qhook-client';
import type { CallbackRequest } from 'qhook-client';

async function example() {
  console.log("🚀 Testing qhook-client SDK...");
  const api = new CallbackApi();

  const body = {
    // string | Callback token (52-character ULID pair)
    token: 01JEXAMPLETOKEN00000000000001JEXAMPLETOKEN0000000000000,
    // { [key: string]: any; }
    requestBody: {"approved":true,"reviewer":"admin@example.com"},
  } satisfies CallbackRequest;

  try {
    const data = await api.callback(body);
    console.log(data);
  } catch (error) {
    console.error(error);
  }
}

// Run the test
example().catch(console.error);
```

### Parameters


| Name | Type | Description  | Notes |
|------------- | ------------- | ------------- | -------------|
| **token** | `string` | Callback token (52-character ULID pair) | [Defaults to `undefined`] |
| **requestBody** | `{ [key: string]: any; }` |  | |

### Return type

[**Callback200Response**](Callback200Response.md)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: `application/json`
- **Accept**: `application/json`


### HTTP response details
| Status code | Description | Response headers |
|-------------|-------------|------------------|
| **200** | Callback received |  -  |
| **404** | Token not found, expired, or already used |  -  |
| **413** | Request body exceeds size limit (default 1MB) |  -  |
| **429** | Per-IP rate limit exceeded |  -  |
| **500** | Internal error |  -  |
| **503** | Concurrency limit exceeded or service unavailable |  -  |

[[Back to top]](#) [[Back to API list]](../README.md#api-endpoints) [[Back to Model list]](../README.md#models) [[Back to README]](../README.md)

