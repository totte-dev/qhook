# qhook_client.ManagementApi

All URIs are relative to *http://localhost:8888*

Method | HTTP request | Description
------------- | ------------- | -------------
[**get_event**](ManagementApi.md#get_event) | **GET** /api/events/{event_id} | Get event details
[**get_job**](ManagementApi.md#get_job) | **GET** /api/jobs/{job_id} | Get job details


# **get_event**
> GetEvent200Response get_event(event_id)

Get event details

Returns event details including associated jobs and workflow runs.
Requires Bearer token authentication if `api.auth_token` is configured.


### Example

* Bearer Authentication (bearerAuth):

```python
import qhook_client
from qhook_client.models.get_event200_response import GetEvent200Response
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

# Configure Bearer authorization: bearerAuth
configuration = qhook_client.Configuration(
    access_token = os.environ["BEARER_TOKEN"]
)

# Enter a context with an instance of the API client
with qhook_client.ApiClient(configuration) as api_client:
    # Create an instance of the API class
    api_instance = qhook_client.ManagementApi(api_client)
    event_id = '01JEXAMPLE00000000000000000' # str | Event ID (ULID)

    try:
        # Get event details
        api_response = api_instance.get_event(event_id)
        print("The response of ManagementApi->get_event:\n")
        pprint(api_response)
    except Exception as e:
        print("Exception when calling ManagementApi->get_event: %s\n" % e)
```



### Parameters


Name | Type | Description  | Notes
------------- | ------------- | ------------- | -------------
 **event_id** | **str**| Event ID (ULID) | 

### Return type

[**GetEvent200Response**](GetEvent200Response.md)

### Authorization

[bearerAuth](../README.md#bearerAuth)

### HTTP request headers

 - **Content-Type**: Not defined
 - **Accept**: application/json, text/plain

### HTTP response details

| Status code | Description | Response headers |
|-------------|-------------|------------------|
**200** | Event details |  -  |
**401** | Missing or invalid Bearer token |  -  |
**404** | Event not found |  -  |

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

# **get_job**
> GetJob200Response get_job(job_id, include_attempts=include_attempts)

Get job details

Returns job details including workflow data. Optionally includes delivery attempts.
Requires Bearer token authentication if `api.auth_token` is configured.


### Example

* Bearer Authentication (bearerAuth):

```python
import qhook_client
from qhook_client.models.get_job200_response import GetJob200Response
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

# Configure Bearer authorization: bearerAuth
configuration = qhook_client.Configuration(
    access_token = os.environ["BEARER_TOKEN"]
)

# Enter a context with an instance of the API client
with qhook_client.ApiClient(configuration) as api_client:
    # Create an instance of the API class
    api_instance = qhook_client.ManagementApi(api_client)
    job_id = '01JEXAMPLE00000000000000000' # str | Job ID (ULID)
    include_attempts = False # bool | Include delivery attempts in response (optional) (default to False)

    try:
        # Get job details
        api_response = api_instance.get_job(job_id, include_attempts=include_attempts)
        print("The response of ManagementApi->get_job:\n")
        pprint(api_response)
    except Exception as e:
        print("Exception when calling ManagementApi->get_job: %s\n" % e)
```



### Parameters


Name | Type | Description  | Notes
------------- | ------------- | ------------- | -------------
 **job_id** | **str**| Job ID (ULID) | 
 **include_attempts** | **bool**| Include delivery attempts in response | [optional] [default to False]

### Return type

[**GetJob200Response**](GetJob200Response.md)

### Authorization

[bearerAuth](../README.md#bearerAuth)

### HTTP request headers

 - **Content-Type**: Not defined
 - **Accept**: application/json, text/plain

### HTTP response details

| Status code | Description | Response headers |
|-------------|-------------|------------------|
**200** | Job details |  -  |
**401** | Missing or invalid Bearer token |  -  |
**404** | Job not found |  -  |

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

