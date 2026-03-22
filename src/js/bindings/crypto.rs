use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rand::Rng;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha384, Sha512};

use crate::js::helpers::{ToSafeCx, create_js_string, js_value_to_string};
use crate::js::{JsResult, JsRuntime};
use mozjs::gc::Handle;
use mozjs::jsapi::{CallArgs, JSContext, JSObject, JSPROP_ENUMERATE};
use mozjs::jsval::{JSVal, UndefinedValue};
use mozjs::rust::wrappers2::JS_DefineFunction;
use std::ffi::CString;
use std::os::raw::c_uint;

/// Install `window.crypto` + `window.crypto.subtle.digest` backed by native Rust primitives.
pub fn setup_crypto(runtime: &mut JsRuntime) -> JsResult<()> {
    runtime.do_with_jsapi(|cx, global| unsafe {
        define_hidden_helper(cx, global, "__stokesCryptoRandomBytes", Some(stokes_crypto_random_bytes), 1)?;
        define_hidden_helper(cx, global, "__stokesCryptoDigestBase64", Some(stokes_crypto_digest_base64), 2)?;
        Ok::<(), String>(())
    })?;

    let script = r#"
        (function() {
            const root = typeof globalThis !== 'undefined'
                ? globalThis
                : (typeof window !== 'undefined' ? window : null);
            if (!root) {
                return;
            }

            const randomNative = root.__stokesCryptoRandomBytes;
            const digestNative = root.__stokesCryptoDigestBase64;
            if (typeof randomNative !== 'function' || typeof digestNative !== 'function') {
                return;
            }

            const BASE64_ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';

            function encodeBase64(bytes) {
                let out = '';
                for (let i = 0; i < bytes.length; i += 3) {
                    const b0 = bytes[i];
                    const b1 = i + 1 < bytes.length ? bytes[i + 1] : 0;
                    const b2 = i + 2 < bytes.length ? bytes[i + 2] : 0;
                    const chunk = (b0 << 16) | (b1 << 8) | b2;

                    out += BASE64_ALPHABET[(chunk >> 18) & 63];
                    out += BASE64_ALPHABET[(chunk >> 12) & 63];
                    out += i + 1 < bytes.length ? BASE64_ALPHABET[(chunk >> 6) & 63] : '=';
                    out += i + 2 < bytes.length ? BASE64_ALPHABET[chunk & 63] : '=';
                }
                return out;
            }

            function decodeBase64(text) {
                if (typeof text !== 'string') {
                    throw new TypeError('Expected base64 string');
                }

                const clean = text.replace(/\s+/g, '');
                if (clean.length % 4 !== 0) {
                    throw new TypeError('Invalid base64 payload length');
                }

                const outputLength = Math.floor((clean.length / 4) * 3) - (clean.endsWith('==') ? 2 : clean.endsWith('=') ? 1 : 0);
                const out = new Uint8Array(outputLength);
                let outIndex = 0;

                for (let i = 0; i < clean.length; i += 4) {
                    const c0 = clean.charAt(i);
                    const c1 = clean.charAt(i + 1);
                    const c2 = clean.charAt(i + 2);
                    const c3 = clean.charAt(i + 3);

                    const v0 = BASE64_ALPHABET.indexOf(c0);
                    const v1 = BASE64_ALPHABET.indexOf(c1);
                    const v2 = c2 === '=' ? 0 : BASE64_ALPHABET.indexOf(c2);
                    const v3 = c3 === '=' ? 0 : BASE64_ALPHABET.indexOf(c3);

                    if (v0 < 0 || v1 < 0 || (c2 !== '=' && v2 < 0) || (c3 !== '=' && v3 < 0)) {
                        throw new TypeError('Invalid base64 payload data');
                    }

                    const chunk = (v0 << 18) | (v1 << 12) | (v2 << 6) | v3;
                    out[outIndex++] = (chunk >> 16) & 255;
                    if (c2 !== '=' && outIndex <= out.length) {
                        out[outIndex++] = (chunk >> 8) & 255;
                    }
                    if (c3 !== '=' && outIndex <= out.length) {
                        out[outIndex++] = chunk & 255;
                    }
                }

                return out;
            }

            function makeDomException(name, message) {
                if (typeof DOMException === 'function') {
                    return new DOMException(message, name);
                }
                const err = new Error(message);
                err.name = name;
                return err;
            }

            function toUint8View(input, opName) {
                if (typeof ArrayBuffer === 'undefined') {
                    throw new TypeError(opName + ': ArrayBuffer is not available');
                }

                if (ArrayBuffer.isView(input)) {
                    return new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
                }

                if (input instanceof ArrayBuffer) {
                    return new Uint8Array(input);
                }

                throw new TypeError(
                    "Failed to execute '" + opName + "' on 'SubtleCrypto': parameter 2 is not of type 'BufferSource'."
                );
            }

            function isIntegerTypedArray(value) {
                if (!value || typeof value !== 'object' || !ArrayBuffer.isView(value) || value instanceof DataView) {
                    return false;
                }

                const tag = Object.prototype.toString.call(value);
                return (
                    tag === '[object Int8Array]' ||
                    tag === '[object Uint8Array]' ||
                    tag === '[object Uint8ClampedArray]' ||
                    tag === '[object Int16Array]' ||
                    tag === '[object Uint16Array]' ||
                    tag === '[object Int32Array]' ||
                    tag === '[object Uint32Array]' ||
                    tag === '[object BigInt64Array]' ||
                    tag === '[object BigUint64Array]'
                );
            }

            function normalizeDigestAlgorithm(algorithm) {
                let name;
                if (typeof algorithm === 'string') {
                    name = algorithm;
                } else if (algorithm && typeof algorithm === 'object' && 'name' in algorithm) {
                    name = algorithm.name;
                }

                if (typeof name !== 'string') {
                    throw new TypeError(
                        "Failed to execute 'digest' on 'SubtleCrypto': 1st argument is not a valid algorithm identifier."
                    );
                }

                const normalized = name.trim().toUpperCase().replace(/_/g, '-');
                if (normalized === 'SHA1') {
                    return 'SHA-1';
                }
                if (normalized === 'SHA256') {
                    return 'SHA-256';
                }
                if (normalized === 'SHA384') {
                    return 'SHA-384';
                }
                if (normalized === 'SHA512') {
                    return 'SHA-512';
                }

                if (normalized === 'SHA-1' || normalized === 'SHA-256' || normalized === 'SHA-384' || normalized === 'SHA-512') {
                    return normalized;
                }

                throw makeDomException('NotSupportedError', "Unrecognized digest algorithm '" + name + "'.");
            }

            class SubtleCryptoImpl {
                digest(algorithm, data) {
                    return Promise.resolve().then(function() {
                        const normalized = normalizeDigestAlgorithm(algorithm);
                        const source = toUint8View(data, 'digest');
                        const payloadBase64 = encodeBase64(source);
                        const digestBase64 = digestNative(normalized, payloadBase64);

                        if (typeof digestBase64 !== 'string') {
                            throw makeDomException('OperationError', 'Failed to compute digest.');
                        }

                        const digestBytes = decodeBase64(digestBase64);
                        return digestBytes.buffer.slice(0);
                    });
                }
            }

            const subtleInstance = new SubtleCryptoImpl();

            class CryptoImpl {
                get subtle() {
                    return subtleInstance;
                }

                getRandomValues(typedArray) {
                    if (!isIntegerTypedArray(typedArray)) {
                        throw new TypeError(
                            "Failed to execute 'getRandomValues' on 'Crypto': The provided value is not an integer typed array."
                        );
                    }

                    const byteLength = typedArray.byteLength;
                    if (byteLength > 65536) {
                        throw makeDomException('QuotaExceededError', 'The requested length exceeds 65536 bytes.');
                    }

                    const randomBase64 = randomNative(byteLength);
                    if (typeof randomBase64 !== 'string') {
                        throw makeDomException('OperationError', 'Unable to read random bytes.');
                    }

                    const randomBytes = decodeBase64(randomBase64);
                    if (randomBytes.length !== byteLength) {
                        throw makeDomException('OperationError', 'Invalid random payload length.');
                    }

                    const target = new Uint8Array(typedArray.buffer, typedArray.byteOffset, byteLength);
                    target.set(randomBytes);
                    return typedArray;
                }

                randomUUID() {
                    const bytes = this.getRandomValues(new Uint8Array(16));
                    bytes[6] = (bytes[6] & 0x0f) | 0x40;
                    bytes[8] = (bytes[8] & 0x3f) | 0x80;

                    const hex = [];
                    for (let i = 0; i < bytes.length; i += 1) {
                        hex.push((bytes[i] + 256).toString(16).slice(1));
                    }

                    return (
                        hex[0] + hex[1] + hex[2] + hex[3] + '-' +
                        hex[4] + hex[5] + '-' +
                        hex[6] + hex[7] + '-' +
                        hex[8] + hex[9] + '-' +
                        hex[10] + hex[11] + hex[12] + hex[13] + hex[14] + hex[15]
                    );
                }
            }

            if (typeof root.SubtleCrypto !== 'function') {
                Object.defineProperty(root, 'SubtleCrypto', {
                    value: SubtleCryptoImpl,
                    writable: true,
                    enumerable: false,
                    configurable: true,
                });
            }

            if (typeof root.Crypto !== 'function') {
                Object.defineProperty(root, 'Crypto', {
                    value: CryptoImpl,
                    writable: true,
                    enumerable: false,
                    configurable: true,
                });
            }

            let cryptoObj = root.crypto;
            if (!cryptoObj || typeof cryptoObj !== 'object') {
                cryptoObj = new CryptoImpl();
            }

            if (typeof cryptoObj.getRandomValues !== 'function') {
                cryptoObj.getRandomValues = CryptoImpl.prototype.getRandomValues;
            }
            if (typeof cryptoObj.randomUUID !== 'function') {
                cryptoObj.randomUUID = CryptoImpl.prototype.randomUUID;
            }
            if (!('subtle' in cryptoObj)) {
                Object.defineProperty(cryptoObj, 'subtle', {
                    get() {
                        return subtleInstance;
                    },
                    enumerable: true,
                    configurable: true,
                });
            }

            Object.defineProperty(root, 'crypto', {
                value: cryptoObj,
                writable: true,
                enumerable: false,
                configurable: true,
            });
        })();
    "#;

    runtime.execute(script, false)
}

unsafe fn define_hidden_helper(
    cx: &mut mozjs::context::JSContext,
    global: Handle<*mut JSObject>,
    name: &str,
    func: mozjs::jsapi::JSNative,
    nargs: u32,
) -> Result<(), String> {
    let cname = CString::new(name).unwrap();
    if JS_DefineFunction(
        cx,
        global.into(),
        cname.as_ptr(),
        func,
        nargs,
        JSPROP_ENUMERATE as u32,
    )
    .is_null()
    {
        Err(format!("Failed to define {} helper", name))
    } else {
        Ok(())
    }
}

unsafe extern "C" fn stokes_crypto_random_bytes(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    if argc < 1 || !args.get(0).is_number() {
        args.rval().set(UndefinedValue());
        return true;
    }

    let requested = args.get(0).to_number();
    if !requested.is_finite() || requested < 0.0 || requested > (usize::MAX as f64) {
        args.rval().set(UndefinedValue());
        return true;
    }

    let len = requested as usize;
    let mut bytes = vec![0u8; len];
    let mut rng = rand::rng();
    rng.fill_bytes(&mut bytes);

    let encoded = STANDARD.encode(bytes);
    let safe_cx = &mut raw_cx.to_safe_cx();
    args.rval().set(create_js_string(safe_cx, &encoded));
    true
}

unsafe extern "C" fn stokes_crypto_digest_base64(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    if argc < 2 {
        args.rval().set(UndefinedValue());
        return true;
    }

    let safe_cx = &mut raw_cx.to_safe_cx();
    let algorithm = js_value_to_string(safe_cx, *args.get(0));
    let payload = js_value_to_string(safe_cx, *args.get(1));

    let Ok(bytes) = STANDARD.decode(payload.as_bytes()) else {
        args.rval().set(UndefinedValue());
        return true;
    };

    let digest_bytes = match normalize_digest_name(&algorithm).as_deref() {
        Some("SHA-1") => {
            let mut hasher = Sha1::new();
            hasher.update(&bytes);
            hasher.finalize().to_vec()
        }
        Some("SHA-256") => {
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            hasher.finalize().to_vec()
        }
        Some("SHA-384") => {
            let mut hasher = Sha384::new();
            hasher.update(&bytes);
            hasher.finalize().to_vec()
        }
        Some("SHA-512") => {
            let mut hasher = Sha512::new();
            hasher.update(&bytes);
            hasher.finalize().to_vec()
        }
        _ => {
            args.rval().set(UndefinedValue());
            return true;
        }
    };

    let encoded = STANDARD.encode(digest_bytes);
    args.rval().set(create_js_string(safe_cx, &encoded));
    true
}

fn normalize_digest_name(name: &str) -> Option<String> {
    let normalized = name.trim().to_ascii_uppercase().replace('_', "-");
    match normalized.as_str() {
        "SHA1" | "SHA-1" => Some("SHA-1".to_string()),
        "SHA256" | "SHA-256" => Some("SHA-256".to_string()),
        "SHA384" | "SHA-384" => Some("SHA-384".to_string()),
        "SHA512" | "SHA-512" => Some("SHA-512".to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_digest_name;

    #[test]
    fn normalize_digest_aliases() {
        assert_eq!(normalize_digest_name("sha1"), Some("SHA-1".to_string()));
        assert_eq!(normalize_digest_name("SHA_256"), Some("SHA-256".to_string()));
        assert_eq!(normalize_digest_name(" sha-384 "), Some("SHA-384".to_string()));
        assert_eq!(normalize_digest_name("Sha512"), Some("SHA-512".to_string()));
        assert_eq!(normalize_digest_name("md5"), None);
    }
}
