
# SendEvent202Response


## Properties

Name | Type
------------ | -------------
`eventId` | string
`jobsCreated` | number

## Example

```typescript
import type { SendEvent202Response } from 'qhook-client'

// TODO: Update the object below with actual values
const example = {
  "eventId": 01JEXAMPLE00000000000000000,
  "jobsCreated": 1,
} satisfies SendEvent202Response

console.log(example)

// Convert the instance to a JSON string
const exampleJSON: string = JSON.stringify(example)
console.log(exampleJSON)

// Parse the JSON string back to an object
const exampleParsed = JSON.parse(exampleJSON) as SendEvent202Response
console.log(exampleParsed)
```

[[Back to top]](#) [[Back to API list]](../README.md#api-endpoints) [[Back to Model list]](../README.md#models) [[Back to README]](../README.md)


