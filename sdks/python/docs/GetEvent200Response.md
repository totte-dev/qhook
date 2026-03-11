# GetEvent200Response


## Properties

Name | Type | Description | Notes
------------ | ------------- | ------------- | -------------
**id** | **str** | Event ULID | 
**source** | **str** | Source name | 
**event_type** | **str** | Event type | 
**payload** | **Dict[str, object]** | Event payload | 
**headers** | **Dict[str, str]** | Stored headers | 
**unique_key** | **str** | Deduplication key | [optional] 
**created_at** | **str** | Creation timestamp (UTC) | 
**jobs** | **List[Dict[str, object]]** | Associated jobs | 
**workflow_runs** | **List[Dict[str, object]]** | Associated workflow runs | 

## Example

```python
from qhook_client.models.get_event200_response import GetEvent200Response

# TODO update the JSON string below
json = "{}"
# create an instance of GetEvent200Response from a JSON string
get_event200_response_instance = GetEvent200Response.from_json(json)
# print the JSON string representation of the object
print(GetEvent200Response.to_json())

# convert the object into a dict
get_event200_response_dict = get_event200_response_instance.to_dict()
# create an instance of GetEvent200Response from a dict
get_event200_response_from_dict = GetEvent200Response.from_dict(get_event200_response_dict)
```
[[Back to Model list]](../README.md#documentation-for-models) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to README]](../README.md)


