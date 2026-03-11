# IngestApi

All URIs are relative to *http://localhost:8888*

| Method | HTTP request | Description |
|------------- | ------------- | -------------|
| [**receiveSns**](IngestApi.md#receivesns) | **POST** /sns/{source} | Receive an AWS SNS message |
| [**receiveWebhook**](IngestApi.md#receivewebhook) | **POST** /webhooks/{source} | Receive a webhook |
| [**sendEvent**](IngestApi.md#sendeventoperation) | **POST** /events/{source}/{event_type} | Send an event |



## receiveSns

> string receiveSns(source, snsMessage)

Receive an AWS SNS message

Receives AWS SNS messages (Notification, SubscriptionConfirmation, UnsubscribeConfirmation). Subscription confirmations are auto-confirmed. Notification payloads are unwrapped from the SNS envelope and processed as events.  X.509 signature verification is performed unless &#x60;skip_verify: true&#x60; is set on the source. 

### Example

```ts
import {
  Configuration,
  IngestApi,
} from 'qhook-client';
import type { ReceiveSnsRequest } from 'qhook-client';

async function example() {
  console.log("🚀 Testing qhook-client SDK...");
  const api = new IngestApi();

  const body = {
    // string | Source name as defined in qhook configuration
    source: github,
    // SnsMessage
    snsMessage: ...,
  } satisfies ReceiveSnsRequest;

  try {
    const data = await api.receiveSns(body);
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
| **source** | `string` | Source name as defined in qhook configuration | [Defaults to `undefined`] |
| **snsMessage** | [SnsMessage](SnsMessage.md) |  | |

### Return type

**string**

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: `application/json`
- **Accept**: `text/plain`


### HTTP response details
| Status code | Description | Response headers |
|-------------|-------------|------------------|
| **200** | Message processed |  -  |
| **400** | Invalid SNS message |  -  |
| **401** | SNS signature verification failed |  -  |
| **404** | Source not found in configuration |  -  |
| **413** | Request body exceeds size limit (default 1MB) |  -  |
| **429** | Per-IP rate limit exceeded |  -  |
| **500** | Internal server error |  -  |
| **503** | Concurrency limit exceeded or service unavailable |  -  |

[[Back to top]](#) [[Back to API list]](../README.md#api-endpoints) [[Back to Model list]](../README.md#models) [[Back to README]](../README.md)


## receiveWebhook

> ReceiveWebhook200Response receiveWebhook(source, requestBody, xHubSignature256, stripeSignature, xShopifyHmacSHA256, ceType, ceSpecversion, ceSource, ceId)

Receive a webhook

Receives a webhook payload from an external provider (e.g. GitHub, Stripe, Shopify). Signature verification is performed if configured for the source.  **Event type extraction order:** 1. &#x60;ce-type&#x60; header (CloudEvents binary mode) 2. &#x60;type&#x60; field in JSON body (CloudEvents structured mode) 3. Provider-specific logic (Stripe &#x60;$.type&#x60;, GitHub &#x60;$.action&#x60;, Shopify &#x60;$.topic&#x60;) 4. Fallback: &#x60;\&quot;event\&quot;&#x60; 

### Example

```ts
import {
  Configuration,
  IngestApi,
} from 'qhook-client';
import type { ReceiveWebhookRequest } from 'qhook-client';

async function example() {
  console.log("🚀 Testing qhook-client SDK...");
  const api = new IngestApi();

  const body = {
    // string | Source name as defined in qhook configuration
    source: github,
    // { [key: string]: any; }
    requestBody: Object,
    // string | GitHub HMAC-SHA256 signature (optional)
    xHubSignature256: sha256=abc123...,
    // string | Stripe signature with timestamp (optional)
    stripeSignature: t=1234567890,v1=abc123...,
    // string | Shopify Base64-encoded HMAC-SHA256 (optional)
    xShopifyHmacSHA256: xShopifyHmacSHA256_example,
    // string | CloudEvents event type (overrides provider-specific extraction) (optional)
    ceType: ceType_example,
    // string | CloudEvents spec version (optional)
    ceSpecversion: 1.0,
    // string | CloudEvents source URI (optional)
    ceSource: ceSource_example,
    // string | CloudEvents event ID (optional)
    ceId: ceId_example,
  } satisfies ReceiveWebhookRequest;

  try {
    const data = await api.receiveWebhook(body);
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
| **source** | `string` | Source name as defined in qhook configuration | [Defaults to `undefined`] |
| **requestBody** | `{ [key: string]: any; }` |  | |
| **xHubSignature256** | `string` | GitHub HMAC-SHA256 signature | [Optional] [Defaults to `undefined`] |
| **stripeSignature** | `string` | Stripe signature with timestamp | [Optional] [Defaults to `undefined`] |
| **xShopifyHmacSHA256** | `string` | Shopify Base64-encoded HMAC-SHA256 | [Optional] [Defaults to `undefined`] |
| **ceType** | `string` | CloudEvents event type (overrides provider-specific extraction) | [Optional] [Defaults to `undefined`] |
| **ceSpecversion** | `string` | CloudEvents spec version | [Optional] [Defaults to `undefined`] |
| **ceSource** | `string` | CloudEvents source URI | [Optional] [Defaults to `undefined`] |
| **ceId** | `string` | CloudEvents event ID | [Optional] [Defaults to `undefined`] |

### Return type

[**ReceiveWebhook200Response**](ReceiveWebhook200Response.md)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: `application/json`, `application/octet-stream`
- **Accept**: `application/json`, `text/plain`


### HTTP response details
| Status code | Description | Response headers |
|-------------|-------------|------------------|
| **200** | Event received or duplicate ignored |  -  |
| **400** | Invalid request |  -  |
| **401** | Signature verification failed |  -  |
| **404** | Source not found in configuration |  -  |
| **413** | Request body exceeds size limit (default 1MB) |  -  |
| **429** | Per-IP rate limit exceeded |  -  |
| **500** | Internal server error |  -  |
| **503** | Concurrency limit exceeded or service unavailable |  -  |

[[Back to top]](#) [[Back to API list]](../README.md#api-endpoints) [[Back to Model list]](../README.md#models) [[Back to README]](../README.md)


## sendEvent

> SendEvent202Response sendEvent(source, eventType, requestBody, ceType, ceSpecversion, ceSource, ceId)

Send an event

Sends an event to qhook with a source name. The source must be defined in the config as &#x60;type: event&#x60;. Multiple event sources are supported (e.g., &#x60;app&#x60;, &#x60;platform&#x60;, &#x60;billing-app&#x60;, &#x60;provisioning-app&#x60;).  If a &#x60;ce-type&#x60; header is present, it overrides the &#x60;event_type&#x60; path parameter. Requires Bearer token authentication if &#x60;api.auth_token&#x60; is configured. 

### Example

```ts
import {
  Configuration,
  IngestApi,
} from 'qhook-client';
import type { SendEventOperationRequest } from 'qhook-client';

async function example() {
  console.log("🚀 Testing qhook-client SDK...");
  const config = new Configuration({ 
    // Configure HTTP bearer authorization: bearerAuth
    accessToken: "YOUR BEARER TOKEN",
  });
  const api = new IngestApi(config);

  const body = {
    // string | Source name as defined in qhook configuration (must be type: event)
    source: platform,
    // string | Event type identifier
    eventType: deploy.start,
    // { [key: string]: any; }
    requestBody: {"order_id":"ORD-123","customer":"cust_456","total":99.99},
    // string | CloudEvents event type (overrides path parameter) (optional)
    ceType: ceType_example,
    // string | CloudEvents spec version (optional)
    ceSpecversion: ceSpecversion_example,
    // string | CloudEvents source URI (optional)
    ceSource: ceSource_example,
    // string | CloudEvents event ID (optional)
    ceId: ceId_example,
  } satisfies SendEventOperationRequest;

  try {
    const data = await api.sendEvent(body);
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
| **source** | `string` | Source name as defined in qhook configuration (must be type: event) | [Defaults to `undefined`] |
| **eventType** | `string` | Event type identifier | [Defaults to `undefined`] |
| **requestBody** | `{ [key: string]: any; }` |  | |
| **ceType** | `string` | CloudEvents event type (overrides path parameter) | [Optional] [Defaults to `undefined`] |
| **ceSpecversion** | `string` | CloudEvents spec version | [Optional] [Defaults to `undefined`] |
| **ceSource** | `string` | CloudEvents source URI | [Optional] [Defaults to `undefined`] |
| **ceId** | `string` | CloudEvents event ID | [Optional] [Defaults to `undefined`] |

### Return type

[**SendEvent202Response**](SendEvent202Response.md)

### Authorization

[bearerAuth](../README.md#bearerAuth)

### HTTP request headers

- **Content-Type**: `application/json`, `application/cloudevents+json`
- **Accept**: `application/json`, `text/plain`


### HTTP response details
| Status code | Description | Response headers |
|-------------|-------------|------------------|
| **202** | Event accepted |  -  |
| **400** | Invalid request |  -  |
| **401** | Missing or invalid Bearer token |  -  |
| **404** | Source not found in configuration |  -  |
| **413** | Request body exceeds size limit (default 1MB) |  -  |
| **429** | Per-IP rate limit exceeded |  -  |
| **500** | Internal server error |  -  |
| **503** | Concurrency limit exceeded or service unavailable |  -  |

[[Back to top]](#) [[Back to API list]](../README.md#api-endpoints) [[Back to Model list]](../README.md#models) [[Back to README]](../README.md)

