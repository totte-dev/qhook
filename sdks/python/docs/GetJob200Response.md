# GetJob200Response


## Properties

Name | Type | Description | Notes
------------ | ------------- | ------------- | -------------
**id** | **str** | Job ULID | 
**event_id** | **str** | Associated event ULID | 
**handler** | **str** | Handler name | 
**url** | **str** | Delivery URL | 
**status** | **str** | Job status | 
**attempt** | **int** | Current attempt number | 
**max_attempts** | **int** | Maximum retry attempts | 
**scheduled_at** | **str** | Next scheduled delivery time (UTC) | 
**last_error** | **str** | Last delivery error message | [optional] 
**workflow_run_id** | **str** | Associated workflow run ID | [optional] 
**step_name** | **str** | Workflow step name | [optional] 
**step_index** | **int** | Workflow step index | [optional] 
**attempts** | **List[Dict[str, object]]** | Delivery attempts (only if include_attempts&#x3D;true) | [optional] 

## Example

```python
from qhook_client.models.get_job200_response import GetJob200Response

# TODO update the JSON string below
json = "{}"
# create an instance of GetJob200Response from a JSON string
get_job200_response_instance = GetJob200Response.from_json(json)
# print the JSON string representation of the object
print(GetJob200Response.to_json())

# convert the object into a dict
get_job200_response_dict = get_job200_response_instance.to_dict()
# create an instance of GetJob200Response from a dict
get_job200_response_from_dict = GetJob200Response.from_dict(get_job200_response_dict)
```
[[Back to Model list]](../README.md#documentation-for-models) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to README]](../README.md)


