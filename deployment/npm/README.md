# @dprint/json

npm distribution of [dprint-plugin-json](https://github.com/dprint/dprint-plugin-json).

Use this with [@dprint/formatter](https://github.com/dprint/js-formatter) or just use @dprint/formatter and download the [dprint-plugin-json WASM file](https://github.com/dprint/dprint-plugin-json/releases).

## Example

```ts
import { createFromBuffer } from "@dprint/formatter";
import { getBuffer } from "@dprint/json";

const formatter = createFromBuffer(getBuffer());

console.log(formatter.formatText("test.json", "{test: 5}"));
```
