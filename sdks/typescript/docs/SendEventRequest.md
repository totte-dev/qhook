
# SendEventRequest


## Properties

Name | Type
------------ | -------------
`specversion` | string
`type` | string
`source` | string
`data` | { [key: string]: any; }

## Example

```typescript
import type { SendEventRequest } from 'qhook-client'

// TODO: Update the object below with actual values
const example = {
  "specversion": 1.0,
  "type": order.created,
  "source": null,
  "data": null,
} satisfies SendEventRequest

console.log(example)

// Convert the instance to a JSON string
const exampleJSON: string = JSON.stringify(example)
console.log(exampleJSON)

// Parse the JSON string back to an object
const exampleParsed = JSON.parse(exampleJSON) as SendEventRequest
console.log(exampleParsed)
```

[[Back to top]](#) [[Back to API list]](../README.md#api-endpoints) [[Back to Model list]](../README.md#models) [[Back to README]](../README.md)


