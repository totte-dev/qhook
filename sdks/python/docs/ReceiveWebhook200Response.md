# ReceiveWebhook200Response


## Properties

Name | Type | Description | Notes
------------ | ------------- | ------------- | -------------
**event_id** | **str** | ULID of the received event | 
**duplicate** | **bool** | Whether the event was a duplicate | 
**jobs_created** | **int** | Number of jobs created for this event | 

## Example

```python
from qhook_client.models.receive_webhook200_response import ReceiveWebhook200Response

# TODO update the JSON string below
json = "{}"
# create an instance of ReceiveWebhook200Response from a JSON string
receive_webhook200_response_instance = ReceiveWebhook200Response.from_json(json)
# print the JSON string representation of the object
print(ReceiveWebhook200Response.to_json())

# convert the object into a dict
receive_webhook200_response_dict = receive_webhook200_response_instance.to_dict()
# create an instance of ReceiveWebhook200Response from a dict
receive_webhook200_response_from_dict = ReceiveWebhook200Response.from_dict(receive_webhook200_response_dict)
```
[[Back to Model list]](../README.md#documentation-for-models) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to README]](../README.md)


