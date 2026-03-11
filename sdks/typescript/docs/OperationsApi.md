# OperationsApi

All URIs are relative to *http://localhost:8888*

| Method | HTTP request | Description |
|------------- | ------------- | -------------|
| [**healthCheck**](OperationsApi.md#healthcheck) | **GET** /health | Health check |
| [**metrics**](OperationsApi.md#metrics) | **GET** /metrics | Prometheus metrics |



## healthCheck

> HealthCheck200Response healthCheck()

Health check

Returns service health status and current queue depth.

### Example

```ts
import {
  Configuration,
  OperationsApi,
} from 'qhook-client';
import type { HealthCheckRequest } from 'qhook-client';

async function example() {
  console.log("🚀 Testing qhook-client SDK...");
  const api = new OperationsApi();

  try {
    const data = await api.healthCheck();
    console.log(data);
  } catch (error) {
    console.error(error);
  }
}

// Run the test
example().catch(console.error);
```

### Parameters

This endpoint does not need any parameter.

### Return type

[**HealthCheck200Response**](HealthCheck200Response.md)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: `application/json`


### HTTP response details
| Status code | Description | Response headers |
|-------------|-------------|------------------|
| **200** | Service is healthy |  -  |
| **429** | Per-IP rate limit exceeded |  -  |
| **503** | Database unreachable |  -  |

[[Back to top]](#) [[Back to API list]](../README.md#api-endpoints) [[Back to Model list]](../README.md#models) [[Back to README]](../README.md)


## metrics

> string metrics()

Prometheus metrics

Returns metrics in Prometheus text exposition format. Requires Bearer token authentication if &#x60;api.metrics_auth_token&#x60; is configured. 

### Example

```ts
import {
  Configuration,
  OperationsApi,
} from 'qhook-client';
import type { MetricsRequest } from 'qhook-client';

async function example() {
  console.log("🚀 Testing qhook-client SDK...");
  const config = new Configuration({ 
    // Configure HTTP bearer authorization: metricsAuth
    accessToken: "YOUR BEARER TOKEN",
  });
  const api = new OperationsApi(config);

  try {
    const data = await api.metrics();
    console.log(data);
  } catch (error) {
    console.error(error);
  }
}

// Run the test
example().catch(console.error);
```

### Parameters

This endpoint does not need any parameter.

### Return type

**string**

### Authorization

[metricsAuth](../README.md#metricsAuth)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: `text/plain`


### HTTP response details
| Status code | Description | Response headers |
|-------------|-------------|------------------|
| **200** | Prometheus metrics |  -  |
| **401** | Missing or invalid metrics auth token |  -  |
| **429** | Per-IP rate limit exceeded |  -  |
| **503** | Concurrency limit exceeded or service unavailable |  -  |

[[Back to top]](#) [[Back to API list]](../README.md#api-endpoints) [[Back to Model list]](../README.md#models) [[Back to README]](../README.md)

