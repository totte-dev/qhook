# SnsMessage


## Properties

Name | Type | Description | Notes
------------ | ------------- | ------------- | -------------
**type** | **str** |  | 
**message_id** | **str** |  | 
**topic_arn** | **str** |  | 
**subject** | **str** |  | [optional] 
**message** | **str** | The actual payload (JSON string for Notification) | 
**timestamp** | **datetime** |  | 
**signature** | **str** |  | [optional] 
**signing_cert_url** | **str** |  | [optional] 
**signature_version** | **str** |  | [optional] 
**subscribe_url** | **str** | Present only for SubscriptionConfirmation | [optional] 

## Example

```python
from qhook_client.models.sns_message import SnsMessage

# TODO update the JSON string below
json = "{}"
# create an instance of SnsMessage from a JSON string
sns_message_instance = SnsMessage.from_json(json)
# print the JSON string representation of the object
print(SnsMessage.to_json())

# convert the object into a dict
sns_message_dict = sns_message_instance.to_dict()
# create an instance of SnsMessage from a dict
sns_message_from_dict = SnsMessage.from_dict(sns_message_dict)
```
[[Back to Model list]](../README.md#documentation-for-models) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to README]](../README.md)


