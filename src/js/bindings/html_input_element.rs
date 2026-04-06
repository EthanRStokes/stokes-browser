use crate::js::JsRuntime;
use tracing::warn;

pub(crate) fn setup_html_input_element_constructor_deferred(runtime: &mut JsRuntime) -> Result<(), String> {
    let script = r#"
        (function() {
            const root = typeof globalThis !== 'undefined' ? globalThis : window;

            function ensureHTMLElementConstructor() {
                if (typeof root.HTMLElement === 'function') {
                    return root.HTMLElement;
                }

                function HTMLElement() {
                    throw new TypeError('Illegal constructor');
                }

                const oldHTMLElement = root.HTMLElement;
                if (oldHTMLElement && typeof oldHTMLElement === 'object') {
                    const keys = Object.getOwnPropertyNames(oldHTMLElement);
                    for (let i = 0; i < keys.length; i++) {
                        const key = keys[i];
                        if (key === 'prototype') {
                            continue;
                        }
                        try {
                            HTMLElement[key] = oldHTMLElement[key];
                        } catch (_) {}
                    }
                }

                root.HTMLElement = HTMLElement;
                return HTMLElement;
            }

            const HTMLElementCtor = ensureHTMLElementConstructor();

            let HTMLInputCtor = root.HTMLInputElement;
            if (typeof HTMLInputCtor !== 'function') {
                HTMLInputCtor = function HTMLInputElement() {
                    throw new TypeError('Illegal constructor');
                };
            }

            const parentProto =
                HTMLElementCtor && HTMLElementCtor.prototype && typeof HTMLElementCtor.prototype === 'object'
                    ? HTMLElementCtor.prototype
                    : Object.prototype;

            if (!HTMLInputCtor.prototype || typeof HTMLInputCtor.prototype !== 'object') {
                HTMLInputCtor.prototype = Object.create(parentProto);
            } else {
                const currentProtoParent = Object.getPrototypeOf(HTMLInputCtor.prototype);
                if (currentProtoParent !== parentProto) {
                    try {
                        Object.setPrototypeOf(HTMLInputCtor.prototype, parentProto);
                    } catch (_) {
                        try {
                            HTMLInputCtor.prototype = Object.create(parentProto);
                        } catch (_2) {}
                    }
                }
            }

            try {
                Object.setPrototypeOf(HTMLInputCtor, HTMLElementCtor);
            } catch (_) {}

            try {
                Object.defineProperty(HTMLInputCtor.prototype, 'constructor', {
                    value: HTMLInputCtor,
                    writable: true,
                    configurable: true,
                    enumerable: false,
                });
            } catch (_) {}

            root.HTMLInputElement = HTMLInputCtor;
        })();
    "#;

    runtime.execute(script, false).map_err(|e| {
        warn!("[JS] Failed to set up HTMLInputElement constructor: {}", e);
        e
    })?;

    Ok(())
}

