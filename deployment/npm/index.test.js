// @ts-check
const assert = require("assert");
const createFromBuffer = require("@dprint/formatter").createFromBuffer;
const getBuffer = require("./index").getBuffer;

const formatter = createFromBuffer(getBuffer());
const result = formatter.formatText("file.json", "{ test: 2, }");

assert.strictEqual(result, "{ \"test\": 2 }\n");
