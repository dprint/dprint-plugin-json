# @dprint/json

npm distribution of [dprint-plugin-json](https://github.com/dprint/dprint-plugin-json).

Use this with [@dprint/formatter](https://github.com/dprint/js-formatter) or just use @dprint/formatter and download the [dprint-plugin-json WASM file](https://github.com/dprint/dprint-plugin-json/releases).

## Example

```ts
import { createFromBuffer } from "@dprint/formatter";
import { getBuffer } from "@dprint/json";
import * as fs from "fs";

const buffer = fs.readFileSync(getPath());
const formatter = createFromBuffer(buffer);

console.log(formatter.formatText("test.json", "{test: 5}"));
```
