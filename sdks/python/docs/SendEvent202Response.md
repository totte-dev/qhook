# SendEvent202Response


## Properties

Name | Type | Description | Notes
------------ | ------------- | ------------- | -------------
**event_id** | **str** | ULID of the accepted event | 
**jobs_created** | **int** | Number of jobs created for this event | 

## Example

```python
from qhook_client.models.send_event202_response import SendEvent202Response

# TODO update the JSON string below
json = "{}"
# create an instance of SendEvent202Response from a JSON string
send_event202_response_instance = SendEvent202Response.from_json(json)
# print the JSON string representation of the object
print(SendEvent202Response.to_json())

# convert the object into a dict
send_event202_response_dict = send_event202_response_instance.to_dict()
# create an instance of SendEvent202Response from a dict
send_event202_response_from_dict = SendEvent202Response.from_dict(send_event202_response_dict)
```
[[Back to Model list]](../README.md#documentation-for-models) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to README]](../README.md)


