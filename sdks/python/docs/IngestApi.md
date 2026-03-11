# qhook_client.IngestApi

All URIs are relative to *http://localhost:8888*

Method | HTTP request | Description
------------- | ------------- | -------------
[**receive_sns**](IngestApi.md#receive_sns) | **POST** /sns/{source} | Receive an AWS SNS message
[**receive_webhook**](IngestApi.md#receive_webhook) | **POST** /webhooks/{source} | Receive a webhook
[**send_event**](IngestApi.md#send_event) | **POST** /events/{source}/{event_type} | Send an event


# **receive_sns**
> str receive_sns(source, sns_message)

Receive an AWS SNS message

Receives AWS SNS messages (Notification, SubscriptionConfirmation, UnsubscribeConfirmation).
Subscription confirmations are auto-confirmed. Notification payloads are unwrapped
from the SNS envelope and processed as events.

X.509 signature verification is performed unless `skip_verify: true` is set on the source.


### Example


```python
import qhook_client
from qhook_client.models.sns_message import SnsMessage
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
    api_instance = qhook_client.IngestApi(api_client)
    source = 'github' # str | Source name as defined in qhook configuration
    sns_message = qhook_client.SnsMessage() # SnsMessage | 

    try:
        # Receive an AWS SNS message
        api_response = api_instance.receive_sns(source, sns_message)
        print("The response of IngestApi->receive_sns:\n")
        pprint(api_response)
    except Exception as e:
        print("Exception when calling IngestApi->receive_sns: %s\n" % e)
```



### Parameters


Name | Type | Description  | Notes
------------- | ------------- | ------------- | -------------
 **source** | **str**| Source name as defined in qhook configuration | 
 **sns_message** | [**SnsMessage**](SnsMessage.md)|  | 

### Return type

**str**

### Authorization

No authorization required

### HTTP request headers

 - **Content-Type**: application/json
 - **Accept**: text/plain

### HTTP response details

| Status code | Description | Response headers |
|-------------|-------------|------------------|
**200** | Message processed |  -  |
**400** | Invalid SNS message |  -  |
**401** | SNS signature verification failed |  -  |
**404** | Source not found in configuration |  -  |
**413** | Request body exceeds size limit (default 1MB) |  -  |
**429** | Per-IP rate limit exceeded |  -  |
**500** | Internal server error |  -  |
**503** | Concurrency limit exceeded or service unavailable |  -  |

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

# **receive_webhook**
> ReceiveWebhook200Response receive_webhook(source, request_body, x_hub_signature_256=x_hub_signature_256, stripe_signature=stripe_signature, x_shopify_hmac_sha256=x_shopify_hmac_sha256, ce_type=ce_type, ce_specversion=ce_specversion, ce_source=ce_source, ce_id=ce_id)

Receive a webhook

Receives a webhook payload from an external provider (e.g. GitHub, Stripe, Shopify).
Signature verification is performed if configured for the source.

**Event type extraction order:**
1. `ce-type` header (CloudEvents binary mode)
2. `type` field in JSON body (CloudEvents structured mode)
3. Provider-specific logic (Stripe `$.type`, GitHub `$.action`, Shopify `$.topic`)
4. Fallback: `"event"`


### Example


```python
import qhook_client
from qhook_client.models.receive_webhook200_response import ReceiveWebhook200Response
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
    api_instance = qhook_client.IngestApi(api_client)
    source = 'github' # str | Source name as defined in qhook configuration
    request_body = None # Dict[str, object] | 
    x_hub_signature_256 = 'sha256=abc123...' # str | GitHub HMAC-SHA256 signature (optional)
    stripe_signature = 't=1234567890,v1=abc123...' # str | Stripe signature with timestamp (optional)
    x_shopify_hmac_sha256 = 'x_shopify_hmac_sha256_example' # str | Shopify Base64-encoded HMAC-SHA256 (optional)
    ce_type = 'ce_type_example' # str | CloudEvents event type (overrides provider-specific extraction) (optional)
    ce_specversion = '1.0' # str | CloudEvents spec version (optional)
    ce_source = 'ce_source_example' # str | CloudEvents source URI (optional)
    ce_id = 'ce_id_example' # str | CloudEvents event ID (optional)

    try:
        # Receive a webhook
        api_response = api_instance.receive_webhook(source, request_body, x_hub_signature_256=x_hub_signature_256, stripe_signature=stripe_signature, x_shopify_hmac_sha256=x_shopify_hmac_sha256, ce_type=ce_type, ce_specversion=ce_specversion, ce_source=ce_source, ce_id=ce_id)
        print("The response of IngestApi->receive_webhook:\n")
        pprint(api_response)
    except Exception as e:
        print("Exception when calling IngestApi->receive_webhook: %s\n" % e)
```



### Parameters


Name | Type | Description  | Notes
------------- | ------------- | ------------- | -------------
 **source** | **str**| Source name as defined in qhook configuration | 
 **request_body** | [**Dict[str, object]**](object.md)|  | 
 **x_hub_signature_256** | **str**| GitHub HMAC-SHA256 signature | [optional] 
 **stripe_signature** | **str**| Stripe signature with timestamp | [optional] 
 **x_shopify_hmac_sha256** | **str**| Shopify Base64-encoded HMAC-SHA256 | [optional] 
 **ce_type** | **str**| CloudEvents event type (overrides provider-specific extraction) | [optional] 
 **ce_specversion** | **str**| CloudEvents spec version | [optional] 
 **ce_source** | **str**| CloudEvents source URI | [optional] 
 **ce_id** | **str**| CloudEvents event ID | [optional] 

### Return type

[**ReceiveWebhook200Response**](ReceiveWebhook200Response.md)

### Authorization

No authorization required

### HTTP request headers

 - **Content-Type**: application/json, application/octet-stream
 - **Accept**: application/json, text/plain

### HTTP response details

| Status code | Description | Response headers |
|-------------|-------------|------------------|
**200** | Event received or duplicate ignored |  -  |
**400** | Invalid request |  -  |
**401** | Signature verification failed |  -  |
**404** | Source not found in configuration |  -  |
**413** | Request body exceeds size limit (default 1MB) |  -  |
**429** | Per-IP rate limit exceeded |  -  |
**500** | Internal server error |  -  |
**503** | Concurrency limit exceeded or service unavailable |  -  |

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

# **send_event**
> SendEvent202Response send_event(source, event_type, request_body, ce_type=ce_type, ce_specversion=ce_specversion, ce_source=ce_source, ce_id=ce_id)

Send an event

Sends an event to qhook with a source name. The source must be defined in
the config as `type: event`. Multiple event sources are supported (e.g.,
`app`, `platform`, `billing-app`, `provisioning-app`).

If a `ce-type` header is present, it overrides the `event_type` path parameter.
Requires Bearer token authentication if `api.auth_token` is configured.


### Example

* Bearer Authentication (bearerAuth):

```python
import qhook_client
from qhook_client.models.send_event202_response import SendEvent202Response
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
    api_instance = qhook_client.IngestApi(api_client)
    source = 'platform' # str | Source name as defined in qhook configuration (must be type: event)
    event_type = 'deploy.start' # str | Event type identifier
    request_body = {"order_id":"ORD-123","customer":"cust_456","total":99.99} # Dict[str, object] | 
    ce_type = 'ce_type_example' # str | CloudEvents event type (overrides path parameter) (optional)
    ce_specversion = 'ce_specversion_example' # str | CloudEvents spec version (optional)
    ce_source = 'ce_source_example' # str | CloudEvents source URI (optional)
    ce_id = 'ce_id_example' # str | CloudEvents event ID (optional)

    try:
        # Send an event
        api_response = api_instance.send_event(source, event_type, request_body, ce_type=ce_type, ce_specversion=ce_specversion, ce_source=ce_source, ce_id=ce_id)
        print("The response of IngestApi->send_event:\n")
        pprint(api_response)
    except Exception as e:
        print("Exception when calling IngestApi->send_event: %s\n" % e)
```



### Parameters


Name | Type | Description  | Notes
------------- | ------------- | ------------- | -------------
 **source** | **str**| Source name as defined in qhook configuration (must be type: event) | 
 **event_type** | **str**| Event type identifier | 
 **request_body** | [**Dict[str, object]**](object.md)|  | 
 **ce_type** | **str**| CloudEvents event type (overrides path parameter) | [optional] 
 **ce_specversion** | **str**| CloudEvents spec version | [optional] 
 **ce_source** | **str**| CloudEvents source URI | [optional] 
 **ce_id** | **str**| CloudEvents event ID | [optional] 

### Return type

[**SendEvent202Response**](SendEvent202Response.md)

### Authorization

[bearerAuth](../README.md#bearerAuth)

### HTTP request headers

 - **Content-Type**: application/json, application/cloudevents+json
 - **Accept**: application/json, text/plain

### HTTP response details

| Status code | Description | Response headers |
|-------------|-------------|------------------|
**202** | Event accepted |  -  |
**400** | Invalid request |  -  |
**401** | Missing or invalid Bearer token |  -  |
**404** | Source not found in configuration |  -  |
**413** | Request body exceeds size limit (default 1MB) |  -  |
**429** | Per-IP rate limit exceeded |  -  |
**500** | Internal server error |  -  |
**503** | Concurrency limit exceeded or service unavailable |  -  |

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

