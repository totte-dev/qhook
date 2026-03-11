
# GetEvent200Response


## Properties

Name | Type
------------ | -------------
`id` | string
`source` | string
`eventType` | string
`payload` | { [key: string]: any; }
`headers` | { [key: string]: string; }
`uniqueKey` | string
`createdAt` | string
`jobs` | Array&lt;{ [key: string]: any; }&gt;
`workflowRuns` | Array&lt;{ [key: string]: any; }&gt;

## Example

```typescript
import type { GetEvent200Response } from 'qhook-client'

// TODO: Update the object below with actual values
const example = {
  "id": null,
  "source": null,
  "eventType": null,
  "payload": null,
  "headers": null,
  "uniqueKey": null,
  "createdAt": null,
  "jobs": null,
  "workflowRuns": null,
} satisfies GetEvent200Response

console.log(example)

// Convert the instance to a JSON string
const exampleJSON: string = JSON.stringify(example)
console.log(exampleJSON)

// Parse the JSON string back to an object
const exampleParsed = JSON.parse(exampleJSON) as GetEvent200Response
console.log(exampleParsed)
```

[[Back to top]](#) [[Back to API list]](../README.md#api-endpoints) [[Back to Model list]](../README.md#models) [[Back to README]](../README.md)


