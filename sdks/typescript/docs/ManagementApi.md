# ManagementApi

All URIs are relative to *http://localhost:8888*

| Method | HTTP request | Description |
|------------- | ------------- | -------------|
| [**getEvent**](ManagementApi.md#getevent) | **GET** /api/events/{event_id} | Get event details |
| [**getJob**](ManagementApi.md#getjob) | **GET** /api/jobs/{job_id} | Get job details |



## getEvent

> GetEvent200Response getEvent(eventId)

Get event details

Returns event details including associated jobs and workflow runs. Requires Bearer token authentication if &#x60;api.auth_token&#x60; is configured. 

### Example

```ts
import {
  Configuration,
  ManagementApi,
} from 'qhook-client';
import type { GetEventRequest } from 'qhook-client';

async function example() {
  console.log("🚀 Testing qhook-client SDK...");
  const config = new Configuration({ 
    // Configure HTTP bearer authorization: bearerAuth
    accessToken: "YOUR BEARER TOKEN",
  });
  const api = new ManagementApi(config);

  const body = {
    // string | Event ID (ULID)
    eventId: 01JEXAMPLE00000000000000000,
  } satisfies GetEventRequest;

  try {
    const data = await api.getEvent(body);
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
| **eventId** | `string` | Event ID (ULID) | [Defaults to `undefined`] |

### Return type

[**GetEvent200Response**](GetEvent200Response.md)

### Authorization

[bearerAuth](../README.md#bearerAuth)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: `application/json`, `text/plain`


### HTTP response details
| Status code | Description | Response headers |
|-------------|-------------|------------------|
| **200** | Event details |  -  |
| **401** | Missing or invalid Bearer token |  -  |
| **404** | Event not found |  -  |

[[Back to top]](#) [[Back to API list]](../README.md#api-endpoints) [[Back to Model list]](../README.md#models) [[Back to README]](../README.md)


## getJob

> GetJob200Response getJob(jobId, includeAttempts)

Get job details

Returns job details including workflow data. Optionally includes delivery attempts. Requires Bearer token authentication if &#x60;api.auth_token&#x60; is configured. 

### Example

```ts
import {
  Configuration,
  ManagementApi,
} from 'qhook-client';
import type { GetJobRequest } from 'qhook-client';

async function example() {
  console.log("🚀 Testing qhook-client SDK...");
  const config = new Configuration({ 
    // Configure HTTP bearer authorization: bearerAuth
    accessToken: "YOUR BEARER TOKEN",
  });
  const api = new ManagementApi(config);

  const body = {
    // string | Job ID (ULID)
    jobId: 01JEXAMPLE00000000000000000,
    // boolean | Include delivery attempts in response (optional)
    includeAttempts: true,
  } satisfies GetJobRequest;

  try {
    const data = await api.getJob(body);
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
| **jobId** | `string` | Job ID (ULID) | [Defaults to `undefined`] |
| **includeAttempts** | `boolean` | Include delivery attempts in response | [Optional] [Defaults to `false`] |

### Return type

[**GetJob200Response**](GetJob200Response.md)

### Authorization

[bearerAuth](../README.md#bearerAuth)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: `application/json`, `text/plain`


### HTTP response details
| Status code | Description | Response headers |
|-------------|-------------|------------------|
| **200** | Job details |  -  |
| **401** | Missing or invalid Bearer token |  -  |
| **404** | Job not found |  -  |

[[Back to top]](#) [[Back to API list]](../README.md#api-endpoints) [[Back to Model list]](../README.md#models) [[Back to README]](../README.md)

