import { leafValue } from "./value.js";

window.__DYN_IMPORT_SIDE_EFFECT__ = "nested-loaded";

export const nestedValue = `nested:${leafValue}`;

