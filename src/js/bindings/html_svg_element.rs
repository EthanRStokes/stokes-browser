use crate::js::JsRuntime;
use tracing::warn;

pub(crate) fn setup_svg_constructors_deferred(runtime: &mut JsRuntime) -> Result<(), String> {
    let script = r#"
        (function() {
            const root = typeof globalThis !== 'undefined' ? globalThis : window;

            function normalizeElementConstructor() {
                if (typeof root.Element === 'function') {
                    return;
                }

                const oldElement = root.Element;
                function Element() {
                    throw new TypeError('Illegal constructor');
                }

                if (oldElement && typeof oldElement === 'object') {
                    const keys = Object.getOwnPropertyNames(oldElement);
                    for (let i = 0; i < keys.length; i++) {
                        const key = keys[i];
                        if (key === 'prototype') {
                            continue;
                        }
                        try {
                            Element[key] = oldElement[key];
                        } catch (_) {
                        }
                    }
                }

                root.Element = Element;
            }

            function ensureConstructor(name, parentName) {
                if (typeof root[name] !== 'function') {
                    root[name] = function() {
                        throw new TypeError('Illegal constructor');
                    };
                }

                const ctor = root[name];
                if (!ctor.prototype || typeof ctor.prototype !== 'object') {
                    ctor.prototype = {};
                }

                if (typeof parentName !== 'string') {
                    return ctor;
                }

                const parentCtor = root[parentName];
                if (typeof parentCtor !== 'function') {
                    return ctor;
                }

                try {
                    Object.setPrototypeOf(ctor, parentCtor);
                    if (parentCtor.prototype && typeof parentCtor.prototype === 'object') {
                        Object.setPrototypeOf(ctor.prototype, parentCtor.prototype);
                    }
                } catch (_) {
                }

                return ctor;
            }

            normalizeElementConstructor();

            if (typeof root.HTMLElement !== 'function') {
                root.HTMLElement = root.Element;
            }

            ensureConstructor('SVGElement', 'Element');
            ensureConstructor('SVGGraphicsElement', 'SVGElement');
            ensureConstructor('SVGGeometryElement', 'SVGGraphicsElement');
            ensureConstructor('SVGSVGElement', 'SVGGraphicsElement');
            ensureConstructor('SVGRectElement', 'SVGGeometryElement');

            if (typeof root.HTMLElement === 'function' && typeof root.Element === 'function') {
                try {
                    Object.setPrototypeOf(root.HTMLElement, root.Element);
                    if (root.Element.prototype && root.HTMLElement.prototype) {
                        Object.setPrototypeOf(root.HTMLElement.prototype, root.Element.prototype);
                    }
                } catch (_) {
                }
            }
        })();
    "#;

    runtime.execute(script, false).map_err(|e| {
        warn!("[JS] Failed to set up SVG constructors: {}", e);
        e
    })?;

    Ok(())
}

