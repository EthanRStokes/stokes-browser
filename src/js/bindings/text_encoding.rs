use crate::js::{JsResult, JsRuntime};

/// Install TextEncoder/TextDecoder polyfills that follow the web API contract.
pub fn setup_text_encoder(runtime: &mut JsRuntime) -> JsResult<()> {
    let script = r#"
        (function() {
            const root = typeof globalThis !== 'undefined'
                ? globalThis
                : (typeof window !== 'undefined' ? window : null);
            if (!root) {
                return;
            }

            function readScalarValue(input, index) {
                const first = input.charCodeAt(index);

                if (first >= 0xD800 && first <= 0xDBFF) {
                    if (index + 1 < input.length) {
                        const second = input.charCodeAt(index + 1);
                        if (second >= 0xDC00 && second <= 0xDFFF) {
                            const codePoint = ((first - 0xD800) << 10) + (second - 0xDC00) + 0x10000;
                            return { codePoint, codeUnits: 2 };
                        }
                    }
                    return { codePoint: 0xFFFD, codeUnits: 1 };
                }

                if (first >= 0xDC00 && first <= 0xDFFF) {
                    return { codePoint: 0xFFFD, codeUnits: 1 };
                }

                return { codePoint: first, codeUnits: 1 };
            }

            function utf8ByteLength(codePoint) {
                if (codePoint <= 0x7F) {
                    return 1;
                }
                if (codePoint <= 0x7FF) {
                    return 2;
                }
                if (codePoint <= 0xFFFF) {
                    return 3;
                }
                return 4;
            }

            function writeUtf8(codePoint, destination, offset) {
                if (codePoint <= 0x7F) {
                    destination[offset] = codePoint;
                    return 1;
                }

                if (codePoint <= 0x7FF) {
                    destination[offset] = 0xC0 | (codePoint >> 6);
                    destination[offset + 1] = 0x80 | (codePoint & 0x3F);
                    return 2;
                }

                if (codePoint <= 0xFFFF) {
                    destination[offset] = 0xE0 | (codePoint >> 12);
                    destination[offset + 1] = 0x80 | ((codePoint >> 6) & 0x3F);
                    destination[offset + 2] = 0x80 | (codePoint & 0x3F);
                    return 3;
                }

                destination[offset] = 0xF0 | (codePoint >> 18);
                destination[offset + 1] = 0x80 | ((codePoint >> 12) & 0x3F);
                destination[offset + 2] = 0x80 | ((codePoint >> 6) & 0x3F);
                destination[offset + 3] = 0x80 | (codePoint & 0x3F);
                return 4;
            }

            function normalizeInput(input) {
                return input === undefined ? '' : String(input);
            }

            function normalizeEncodingLabel(label) {
                if (label === undefined) {
                    return 'utf-8';
                }
                return String(label).trim().toLowerCase();
            }

            function canonicalEncoding(label) {
                const normalized = normalizeEncodingLabel(label);
                if (
                    normalized === 'utf-8' ||
                    normalized === 'utf8' ||
                    normalized === 'unicode-1-1-utf-8'
                ) {
                    return 'utf-8';
                }
                throw new RangeError(
                    "Failed to construct 'TextDecoder': The encoding label provided ('" + normalized + "') is invalid."
                );
            }

            function toUint8View(input) {
                if (input === undefined) {
                    return new Uint8Array(0);
                }
                if (input instanceof Uint8Array) {
                    return input;
                }
                if (typeof ArrayBuffer !== 'undefined' && input instanceof ArrayBuffer) {
                    return new Uint8Array(input);
                }
                if (typeof ArrayBuffer !== 'undefined' && typeof ArrayBuffer.isView === 'function' && ArrayBuffer.isView(input)) {
                    return new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
                }
                throw new TypeError(
                    "Failed to execute 'decode' on 'TextDecoder': parameter 1 is not of type 'ArrayBuffer, BufferSource, or view'."
                );
            }

            function appendCodePoint(chars, codePoint) {
                if (codePoint <= 0xFFFF) {
                    chars.push(String.fromCharCode(codePoint));
                    return;
                }
                const adjusted = codePoint - 0x10000;
                chars.push(String.fromCharCode(0xD800 + (adjusted >> 10)));
                chars.push(String.fromCharCode(0xDC00 + (adjusted & 0x3FF)));
            }

            function decodeUtf8Chunk(bytes, fatal, stream, pendingBytes) {
                const chars = [];
                const allBytes = pendingBytes.length
                    ? pendingBytes.concat(Array.prototype.slice.call(bytes))
                    : Array.prototype.slice.call(bytes);

                let i = 0;
                const len = allBytes.length;
                const nextPending = [];

                function emitReplacement() {
                    if (fatal) {
                        throw new TypeError("The encoded data was not valid for encoding utf-8");
                    }
                    chars.push('\uFFFD');
                }

                while (i < len) {
                    const first = allBytes[i];
                    if (first <= 0x7F) {
                        appendCodePoint(chars, first);
                        i += 1;
                        continue;
                    }

                    let needed = 0;
                    let minCodePoint = 0;
                    let codePoint = 0;

                    if (first >= 0xC2 && first <= 0xDF) {
                        needed = 1;
                        minCodePoint = 0x80;
                        codePoint = first & 0x1F;
                    } else if (first >= 0xE0 && first <= 0xEF) {
                        needed = 2;
                        minCodePoint = 0x800;
                        codePoint = first & 0x0F;
                    } else if (first >= 0xF0 && first <= 0xF4) {
                        needed = 3;
                        minCodePoint = 0x10000;
                        codePoint = first & 0x07;
                    } else {
                        emitReplacement();
                        i += 1;
                        continue;
                    }

                    if (i + needed >= len) {
                        if (stream) {
                            for (let j = i; j < len; j += 1) {
                                nextPending.push(allBytes[j]);
                            }
                            i = len;
                            continue;
                        }
                        emitReplacement();
                        i = len;
                        continue;
                    }

                    let valid = true;
                    for (let j = 1; j <= needed; j += 1) {
                        const cont = allBytes[i + j];
                        if ((cont & 0xC0) !== 0x80) {
                            valid = false;
                            break;
                        }
                        codePoint = (codePoint << 6) | (cont & 0x3F);
                    }

                    if (
                        !valid ||
                        codePoint < minCodePoint ||
                        codePoint > 0x10FFFF ||
                        (codePoint >= 0xD800 && codePoint <= 0xDFFF)
                    ) {
                        emitReplacement();
                        i += 1;
                        continue;
                    }

                    appendCodePoint(chars, codePoint);
                    i += needed + 1;
                }

                return {
                    text: chars.join(''),
                    pending: nextPending,
                };
            }

            if (typeof root.TextEncoder !== 'function') {
                class TextEncoder {
                    constructor() {}

                    get encoding() {
                        return 'utf-8';
                    }

                    encode(input = '') {
                        const source = normalizeInput(input);

                        let totalBytes = 0;
                        for (let i = 0; i < source.length;) {
                            const scalar = readScalarValue(source, i);
                            totalBytes += utf8ByteLength(scalar.codePoint);
                            i += scalar.codeUnits;
                        }

                        const output = new Uint8Array(totalBytes);
                        let written = 0;
                        for (let i = 0; i < source.length;) {
                            const scalar = readScalarValue(source, i);
                            written += writeUtf8(scalar.codePoint, output, written);
                            i += scalar.codeUnits;
                        }

                        return output;
                    }

                    encodeInto(input, destination) {
                        if (!(destination instanceof Uint8Array)) {
                            throw new TypeError(
                                "Failed to execute 'encodeInto' on 'TextEncoder': parameter 2 is not of type 'Uint8Array'."
                            );
                        }

                        const source = normalizeInput(input);
                        let read = 0;
                        let written = 0;

                        for (let i = 0; i < source.length;) {
                            const scalar = readScalarValue(source, i);
                            const needed = utf8ByteLength(scalar.codePoint);
                            if (written + needed > destination.length) {
                                break;
                            }

                            written += writeUtf8(scalar.codePoint, destination, written);
                            i += scalar.codeUnits;
                            read += scalar.codeUnits;
                        }

                        return { read, written };
                    }
                }

                if (typeof Symbol !== 'undefined' && Symbol.toStringTag) {
                    Object.defineProperty(TextEncoder.prototype, Symbol.toStringTag, {
                        value: 'TextEncoder',
                        writable: false,
                        enumerable: false,
                        configurable: true,
                    });
                }

                Object.defineProperty(root, 'TextEncoder', {
                    value: TextEncoder,
                    writable: true,
                    enumerable: false,
                    configurable: true,
                });
            }

            if (typeof root.TextDecoder !== 'function') {
                class TextDecoder {
                    constructor(label = 'utf-8', options = {}) {
                        this._encoding = canonicalEncoding(label);
                        this._fatal = !!(options && options.fatal);
                        this._ignoreBOM = !!(options && options.ignoreBOM);
                        this._pendingBytes = [];
                        this._bomHandled = false;
                    }

                    get encoding() {
                        return this._encoding;
                    }

                    get fatal() {
                        return this._fatal;
                    }

                    get ignoreBOM() {
                        return this._ignoreBOM;
                    }

                    decode(input, options = {}) {
                        const bytes = toUint8View(input);
                        const stream = !!(options && options.stream);

                        const decoded = decodeUtf8Chunk(bytes, this._fatal, stream, this._pendingBytes);
                        this._pendingBytes = decoded.pending;
                        let text = decoded.text;

                        if (!stream && this._pendingBytes.length) {
                            if (this._fatal) {
                                throw new TypeError('The encoded data was not valid for encoding utf-8');
                            }
                            text += '\uFFFD';
                            this._pendingBytes = [];
                        }

                        if (!this._ignoreBOM && !this._bomHandled && text.length) {
                            if (text.charCodeAt(0) === 0xFEFF) {
                                text = text.slice(1);
                            }
                            this._bomHandled = true;
                        } else if (!this._bomHandled && bytes.length > 0 && !stream) {
                            this._bomHandled = true;
                        }

                        if (!stream) {
                            this._pendingBytes = [];
                        }

                        return text;
                    }
                }

                if (typeof Symbol !== 'undefined' && Symbol.toStringTag) {
                    Object.defineProperty(TextDecoder.prototype, Symbol.toStringTag, {
                        value: 'TextDecoder',
                        writable: false,
                        enumerable: false,
                        configurable: true,
                    });
                }

                Object.defineProperty(root, 'TextDecoder', {
                    value: TextDecoder,
                    writable: true,
                    enumerable: false,
                    configurable: true,
                });
            }
        })();
    "#;

    runtime.execute(script, false)
}


