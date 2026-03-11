
# SnsMessage


## Properties

Name | Type
------------ | -------------
`type` | string
`messageId` | string
`topicArn` | string
`subject` | string
`message` | string
`timestamp` | Date
`signature` | string
`signingCertURL` | string
`signatureVersion` | string
`subscribeURL` | string

## Example

```typescript
import type { SnsMessage } from 'qhook-client'

// TODO: Update the object below with actual values
const example = {
  "type": null,
  "messageId": null,
  "topicArn": null,
  "subject": null,
  "message": null,
  "timestamp": null,
  "signature": null,
  "signingCertURL": null,
  "signatureVersion": null,
  "subscribeURL": null,
} satisfies SnsMessage

console.log(example)

// Convert the instance to a JSON string
const exampleJSON: string = JSON.stringify(example)
console.log(exampleJSON)

// Parse the JSON string back to an object
const exampleParsed = JSON.parse(exampleJSON) as SnsMessage
console.log(exampleParsed)
```

[[Back to top]](#) [[Back to API list]](../README.md#api-endpoints) [[Back to Model list]](../README.md#models) [[Back to README]](../README.md)


