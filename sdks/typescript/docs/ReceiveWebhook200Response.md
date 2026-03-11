
# ReceiveWebhook200Response


## Properties

Name | Type
------------ | -------------
`eventId` | string
`duplicate` | boolean
`jobsCreated` | number

## Example

```typescript
import type { ReceiveWebhook200Response } from 'qhook-client'

// TODO: Update the object below with actual values
const example = {
  "eventId": 01JEXAMPLE00000000000000000,
  "duplicate": false,
  "jobsCreated": 2,
} satisfies ReceiveWebhook200Response

console.log(example)

// Convert the instance to a JSON string
const exampleJSON: string = JSON.stringify(example)
console.log(exampleJSON)

// Parse the JSON string back to an object
const exampleParsed = JSON.parse(exampleJSON) as ReceiveWebhook200Response
console.log(exampleParsed)
```

[[Back to top]](#) [[Back to API list]](../README.md#api-endpoints) [[Back to Model list]](../README.md#models) [[Back to README]](../README.md)


