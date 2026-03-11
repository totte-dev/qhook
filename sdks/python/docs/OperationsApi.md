# qhook_client.OperationsApi

All URIs are relative to *http://localhost:8888*

Method | HTTP request | Description
------------- | ------------- | -------------
[**health_check**](OperationsApi.md#health_check) | **GET** /health | Health check
[**metrics**](OperationsApi.md#metrics) | **GET** /metrics | Prometheus metrics


# **health_check**
> HealthCheck200Response health_check()

Health check

Returns service health status and current queue depth.

### Example


```python
import qhook_client
from qhook_client.models.health_check200_response import HealthCheck200Response
from qhook_client.rest import ApiException
from pprint import pprint

# Defining the host is optional and defaults to http://localhost:8888
# See configuration.py for a list of all supported configuration parameters.
configuration = qhook_client.Configuration(
    host = "http://localhost:8888"
)


# Enter a context with an instance of the API client
with qhook_client.ApiClient(configuration) as api_client:
    # Create an instance of the API class
    api_instance = qhook_client.OperationsApi(api_client)

    try:
        # Health check
        api_response = api_instance.health_check()
        print("The response of OperationsApi->health_check:\n")
        pprint(api_response)
    except Exception as e:
        print("Exception when calling OperationsApi->health_check: %s\n" % e)
```



### Parameters

This endpoint does not need any parameter.

### Return type

[**HealthCheck200Response**](HealthCheck200Response.md)

### Authorization

No authorization required

### HTTP request headers

 - **Content-Type**: Not defined
 - **Accept**: application/json

### HTTP response details

| Status code | Description | Response headers |
|-------------|-------------|------------------|
**200** | Service is healthy |  -  |
**429** | Per-IP rate limit exceeded |  -  |
**503** | Database unreachable |  -  |

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

# **metrics**
> str metrics()

Prometheus metrics

Returns metrics in Prometheus text exposition format.
Requires Bearer token authentication if `api.metrics_auth_token` is configured.


### Example

* Bearer Authentication (metricsAuth):

```python
import qhook_client
from qhook_client.rest import ApiException
from pprint import pprint

# Defining the host is optional and defaults to http://localhost:8888
# See configuration.py for a list of all supported configuration parameters.
configuration = qhook_client.Configuration(
    host = "http://localhost:8888"
)

# The client must configure the authentication and authorization parameters
# in accordance with the API server security policy.
# Examples for each auth method are provided below, use the example that
# satisfies your auth use case.

# Configure Bearer authorization: metricsAuth
configuration = qhook_client.Configuration(
    access_token = os.environ["BEARER_TOKEN"]
)

# Enter a context with an instance of the API client
with qhook_client.ApiClient(configuration) as api_client:
    # Create an instance of the API class
    api_instance = qhook_client.OperationsApi(api_client)

    try:
        # Prometheus metrics
        api_response = api_instance.metrics()
        print("The response of OperationsApi->metrics:\n")
        pprint(api_response)
    except Exception as e:
        print("Exception when calling OperationsApi->metrics: %s\n" % e)
```



### Parameters

This endpoint does not need any parameter.

### Return type

**str**

### Authorization

[metricsAuth](../README.md#metricsAuth)

### HTTP request headers

 - **Content-Type**: Not defined
 - **Accept**: text/plain

### HTTP response details

| Status code | Description | Response headers |
|-------------|-------------|------------------|
**200** | Prometheus metrics |  -  |
**401** | Missing or invalid metrics auth token |  -  |
**429** | Per-IP rate limit exceeded |  -  |
**503** | Concurrency limit exceeded or service unavailable |  -  |

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

