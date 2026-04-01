import { nestedValue } from "./nested.js";

export const answer = 42;

export function describe() {
    return `entry:${nestedValue}:${answer}`;
}

