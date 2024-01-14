import { generateChangeLog } from "https://raw.githubusercontent.com/dprint/automation/0.9.0/changelog.ts";

const version = Deno.args[0];
const changelog = await generateChangeLog({
  versionTo: version,
});
const text = `## Changes

${changelog}

## Install

[Install](https://dprint.dev/install/) and [setup](https://dprint.dev/setup/) dprint.

Then in your project's dprint configuration file:

1. Specify the plugin url in the \`"plugins"\` array (can be done via \`dprint config add json\`).
2. Add a \`"json"\` configuration property if desired.
   \`\`\`jsonc
   {
     // ...etc...
     "json": {
       // json config goes here
     },
     "plugins": [
       "https://plugins.dprint.dev/json-${version}.wasm"
     ]
   }
   \`\`\`

## JS Formatting API

* [JS Formatter](https://github.com/dprint/js-formatter) - Browser/Deno and Node
* [npm package](https://www.npmjs.com/package/@dprint/json)
`;

console.log(text);
