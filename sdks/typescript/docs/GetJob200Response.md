
# GetJob200Response


## Properties

Name | Type
------------ | -------------
`id` | string
`eventId` | string
`handler` | string
`url` | string
`status` | string
`attempt` | number
`maxAttempts` | number
`scheduledAt` | string
`lastError` | string
`workflowRunId` | string
`stepName` | string
`stepIndex` | number
`attempts` | Array&lt;{ [key: string]: any; }&gt;

## Example

```typescript
import type { GetJob200Response } from 'qhook-client'

// TODO: Update the object below with actual values
const example = {
  "id": null,
  "eventId": null,
  "handler": null,
  "url": null,
  "status": null,
  "attempt": null,
  "maxAttempts": null,
  "scheduledAt": null,
  "lastError": null,
  "workflowRunId": null,
  "stepName": null,
  "stepIndex": null,
  "attempts": null,
} satisfies GetJob200Response

console.log(example)

// Convert the instance to a JSON string
const exampleJSON: string = JSON.stringify(example)
console.log(exampleJSON)

// Parse the JSON string back to an object
const exampleParsed = JSON.parse(exampleJSON) as GetJob200Response
console.log(exampleParsed)
```

[[Back to top]](#) [[Back to API list]](../README.md#api-endpoints) [[Back to Model list]](../README.md#models) [[Back to README]](../README.md)


