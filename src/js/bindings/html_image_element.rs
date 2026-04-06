use crate::js::JsRuntime;
use tracing::warn;

pub(crate) fn setup_image_constructor_deferred(runtime: &mut JsRuntime) -> Result<(), String> {
    let script = r#"
        (function () {
            function Image(width, height) {
                var img = document.createElement('img');
                if (width !== undefined) {
                    var w = +width || 0;
                    img.width = w;
                    img.setAttribute('width', String(w));
                }
                if (height !== undefined) {
                    var h = +height || 0;
                    img.height = h;
                    img.setAttribute('height', String(h));
                }

                img.naturalWidth = 0;
                img.naturalHeight = 0;
                img.complete = false;
                img.onload = null;
                img.onerror = null;
                img.onabort = null;
                img.alt = '';
                img.crossOrigin = null;
                img.decoding = 'auto';
                img.loading = 'eager';
                img.referrerPolicy = '';
                img.isMap = false;
                img.useMap = '';

                var _src = '';
                Object.defineProperty(img, 'src', {
                    get: function () { return _src; },
                    set: function (url) {
                        var strUrl = String(url == null ? '' : url);
                        _src = strUrl;
                        try { img.setAttribute('src', strUrl); } catch (_e) {}
                        if (!strUrl) {
                            img.complete = true;
                            return;
                        }
                        try {
                            fetch(strUrl)
                                .then(function (response) {
                                    img.complete = true;
                                    if (response.ok) {
                                        if (typeof img.onload === 'function') {
                                            try {
                                                img.onload.call(img, { type: 'load', target: img, currentTarget: img });
                                            } catch (_e) {}
                                        }
                                    } else {
                                        if (typeof img.onerror === 'function') {
                                            try {
                                                img.onerror.call(img, { type: 'error', target: img, currentTarget: img });
                                            } catch (_e) {}
                                        }
                                    }
                                })
                                .catch(function () {
                                    img.complete = true;
                                    if (typeof img.onerror === 'function') {
                                        try {
                                            img.onerror.call(img, { type: 'error', target: img, currentTarget: img });
                                        } catch (_e) {}
                                    }
                                });
                        } catch (_e) {
                            img.complete = true;
                            if (typeof img.onerror === 'function') {
                                try {
                                    img.onerror.call(img, { type: 'error', target: img, currentTarget: img });
                                } catch (_e2) {}
                            }
                        }
                    },
                    configurable: true,
                    enumerable: true
                });

                return img;
            }

            globalThis.Image = Image;
            globalThis.HTMLImageElement = Image;
        })();
    "#;

    runtime.execute(script, false).map_err(|e| {
        warn!("[JS] Failed to set up Image constructor: {}", e);
        e
    })?;

    Ok(())
}

