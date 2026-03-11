# qhook_client.CallbackApi

All URIs are relative to *http://localhost:8888*

Method | HTTP request | Description
------------- | ------------- | -------------
[**callback**](CallbackApi.md#callback) | **POST** /callback/{token} | Resume a waiting workflow step


# **callback**
> Callback200Response callback(token, request_body)

Resume a waiting workflow step

Resumes a workflow step that is waiting for a callback. The token is a
cryptographic identifier generated when the callback step was created.

The request body becomes the step output, available to subsequent steps
via response chaining.

Returns a uniform 404 for all failure cases (invalid, expired, or
already-used tokens) to prevent token enumeration.


### Example


```python
import qhook_client
from qhook_client.models.callback200_response import Callback200Response
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
    api_instance = qhook_client.CallbackApi(api_client)
    token = '01JEXAMPLETOKEN00000000000001JEXAMPLETOKEN0000000000000' # str | Callback token (52-character ULID pair)
    request_body = {"approved":true,"reviewer":"admin@example.com"} # Dict[str, object] | 

    try:
        # Resume a waiting workflow step
        api_response = api_instance.callback(token, request_body)
        print("The response of CallbackApi->callback:\n")
        pprint(api_response)
    except Exception as e:
        print("Exception when calling CallbackApi->callback: %s\n" % e)
```



### Parameters


Name | Type | Description  | Notes
------------- | ------------- | ------------- | -------------
 **token** | **str**| Callback token (52-character ULID pair) | 
 **request_body** | [**Dict[str, object]**](object.md)|  | 

### Return type

[**Callback200Response**](Callback200Response.md)

### Authorization

No authorization required

### HTTP request headers

 - **Content-Type**: application/json
 - **Accept**: application/json

### HTTP response details

| Status code | Description | Response headers |
|-------------|-------------|------------------|
**200** | Callback received |  -  |
**404** | Token not found, expired, or already used |  -  |
**413** | Request body exceeds size limit (default 1MB) |  -  |
**429** | Per-IP rate limit exceeded |  -  |
**500** | Internal error |  -  |
**503** | Concurrency limit exceeded or service unavailable |  -  |

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

