// XMLHttpRequest implementation for JavaScript
// Implemented as a JavaScript polyfill that uses the native fetch() API under the hood.

use crate::js::JsRuntime;

/// Set up the XMLHttpRequest constructor in the JavaScript global scope.
///
/// XMLHttpRequest is implemented as a pure-JavaScript polyfill on top of the
/// browser's native `fetch()` binding so that all network I/O goes through the
/// same curl-backed code path.
pub fn setup_xhr(runtime: &mut JsRuntime) -> Result<(), String> {
    let script = r#"
(function () {
    'use strict';

    // -----------------------------------------------------------------------
    // XMLHttpRequest
    // -----------------------------------------------------------------------

    var UNSENT           = 0;
    var OPENED           = 1;
    var HEADERS_RECEIVED = 2;
    var LOADING          = 3;
    var DONE             = 4;

    function XMLHttpRequest() {
        // Ready state
        this.readyState = UNSENT;

        // Response
        this.status      = 0;
        this.statusText  = '';
        this.response    = null;
        this.responseText = '';
        this.responseXML  = null;
        this.responseURL  = '';
        this.responseType = '';

        // Config
        this.timeout        = 0;
        this.withCredentials = false;

        // Upload object (stub – most sites just check it exists)
        this.upload = (function () {
            var u = {};
            u._listeners = {};
            u.addEventListener = function (type, fn) {
                if (!u._listeners[type]) u._listeners[type] = [];
                u._listeners[type].push(fn);
            };
            u.removeEventListener = function (type, fn) {
                if (!u._listeners[type]) return;
                var idx = u._listeners[type].indexOf(fn);
                if (idx !== -1) u._listeners[type].splice(idx, 1);
            };
            return u;
        }());

        // Event handlers (IDL attributes)
        this.onreadystatechange = null;
        this.onload    = null;
        this.onerror   = null;
        this.onabort   = null;
        this.ontimeout = null;
        this.onprogress   = null;
        this.onloadstart  = null;
        this.onloadend    = null;

        // Internal state
        this._method          = '';
        this._url             = '';
        this._async           = true;
        this._user            = null;
        this._password        = null;
        this._requestHeaders  = {};
        this._responseHeaders = {};
        this._aborted         = false;
        this._sent            = false;
        this._listeners       = {};
        this._overriddenMime  = null;
    }

    // Static constants
    XMLHttpRequest.UNSENT           = UNSENT;
    XMLHttpRequest.OPENED           = OPENED;
    XMLHttpRequest.HEADERS_RECEIVED = HEADERS_RECEIVED;
    XMLHttpRequest.LOADING          = LOADING;
    XMLHttpRequest.DONE             = DONE;

    // Prototype constants
    XMLHttpRequest.prototype.UNSENT           = UNSENT;
    XMLHttpRequest.prototype.OPENED           = OPENED;
    XMLHttpRequest.prototype.HEADERS_RECEIVED = HEADERS_RECEIVED;
    XMLHttpRequest.prototype.LOADING          = LOADING;
    XMLHttpRequest.prototype.DONE             = DONE;

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    XMLHttpRequest.prototype._setReadyState = function (state) {
        this.readyState = state;
        this._fireEvent('readystatechange', {});
    };

    XMLHttpRequest.prototype._fireEvent = function (type, extra) {
        var event = { type: type, target: this, currentTarget: this, bubbles: false, cancelable: false };
        // Merge extra fields
        for (var k in extra) {
            if (Object.prototype.hasOwnProperty.call(extra, k)) event[k] = extra[k];
        }

        // Call IDL handler
        var idl = this['on' + type];
        if (typeof idl === 'function') {
            try { idl.call(this, event); } catch (_e) {}
        }

        // Call registered listeners
        var list = this._listeners[type];
        if (list) {
            // Copy the list so removals during iteration are safe
            var copy = list.slice();
            for (var i = 0; i < copy.length; i++) {
                try { copy[i].call(this, event); } catch (_e) {}
            }
        }
    };

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    XMLHttpRequest.prototype.addEventListener = function (type, handler) {
        if (typeof handler !== 'function') return;
        if (!this._listeners[type]) this._listeners[type] = [];
        if (this._listeners[type].indexOf(handler) === -1) {
            this._listeners[type].push(handler);
        }
    };

    XMLHttpRequest.prototype.removeEventListener = function (type, handler) {
        if (!this._listeners[type]) return;
        var idx = this._listeners[type].indexOf(handler);
        if (idx !== -1) this._listeners[type].splice(idx, 1);
    };

    XMLHttpRequest.prototype.dispatchEvent = function (event) {
        var type = event.type;
        var idl = this['on' + type];
        if (typeof idl === 'function') {
            try { idl.call(this, event); } catch (_e) {}
        }
        var list = this._listeners[type];
        if (list) {
            var copy = list.slice();
            for (var i = 0; i < copy.length; i++) {
                try { copy[i].call(this, event); } catch (_e) {}
            }
        }
        return true;
    };

    XMLHttpRequest.prototype.open = function (method, url, async, user, password) {
        this._method   = String(method).toUpperCase();
        this._url      = String(url);
        this._async    = (async !== false);   // defaults to true per spec
        this._user     = (user     != null) ? String(user)     : null;
        this._password = (password != null) ? String(password) : null;

        this._requestHeaders  = {};
        this._responseHeaders = {};
        this._aborted = false;
        this._sent    = false;

        this.status      = 0;
        this.statusText  = '';
        this.response    = null;
        this.responseText = '';
        this.responseURL  = '';
        this.responseXML  = null;

        this._setReadyState(OPENED);
    };

    XMLHttpRequest.prototype.setRequestHeader = function (name, value) {
        if (this.readyState < OPENED) {
            throw new DOMException('Failed to execute \'setRequestHeader\': The object\'s state must be OPENED.', 'InvalidStateError');
        }
        var lower = String(name).toLowerCase();
        if (Object.prototype.hasOwnProperty.call(this._requestHeaders, lower)) {
            this._requestHeaders[lower] += ', ' + String(value);
        } else {
            this._requestHeaders[lower] = String(value);
        }
    };

    XMLHttpRequest.prototype.getResponseHeader = function (name) {
        var lower = String(name).toLowerCase();
        if (Object.prototype.hasOwnProperty.call(this._responseHeaders, lower)) {
            return this._responseHeaders[lower];
        }
        return null;
    };

    XMLHttpRequest.prototype.getAllResponseHeaders = function () {
        if (this.readyState < HEADERS_RECEIVED) return '';
        var result = '';
        for (var key in this._responseHeaders) {
            if (Object.prototype.hasOwnProperty.call(this._responseHeaders, key)) {
                result += key + ': ' + this._responseHeaders[key] + '\r\n';
            }
        }
        return result;
    };

    XMLHttpRequest.prototype.overrideMimeType = function (mimeType) {
        this._overriddenMime = String(mimeType);
    };

    XMLHttpRequest.prototype.abort = function () {
        this._aborted = true;
        if (this.readyState > UNSENT && this.readyState < DONE) {
            this.status     = 0;
            this.statusText = '';
            this._setReadyState(DONE);
            this._fireEvent('abort', { loaded: 0, total: 0 });
            this._fireEvent('loadend', { loaded: 0, total: 0 });
        }
        this.readyState = UNSENT;
    };

    XMLHttpRequest.prototype.send = function (body) {
        if (this.readyState !== OPENED) {
            throw new DOMException('Failed to execute \'send\': The object\'s state must be OPENED.', 'InvalidStateError');
        }
        if (this._sent) {
            throw new DOMException('Failed to execute \'send\': Already sent.', 'InvalidStateError');
        }

        this._sent = true;
        var self = this;

        // Build fetch options
        var options = {
            method: this._method,
            headers: {}
        };

        // Copy request headers
        for (var h in this._requestHeaders) {
            if (Object.prototype.hasOwnProperty.call(this._requestHeaders, h)) {
                options.headers[h] = this._requestHeaders[h];
            }
        }

        // Attach body for methods that carry one
        if (body !== undefined && body !== null &&
            this._method !== 'GET' && this._method !== 'HEAD') {
            options.body = String(body);
        }

        // Fire loadstart
        self._fireEvent('loadstart', { loaded: 0, total: 0 });

        try {
            fetch(this._url, options)
                .then(function (response) {
                    if (self._aborted) return;

                    // Copy response headers (they are stored as plain object properties
                    // in our fetch polyfill)
                    var hdrs = response.headers;
                    if (hdrs && typeof hdrs === 'object') {
                        for (var k in hdrs) {
                            if (Object.prototype.hasOwnProperty.call(hdrs, k)) {
                                self._responseHeaders[k.toLowerCase()] = hdrs[k];
                            }
                        }
                    }

                    self.responseURL = response.url || self._url;

                    // HEADERS_RECEIVED
                    self.status     = response.status     || 0;
                    self.statusText = response.statusText || '';
                    self._setReadyState(HEADERS_RECEIVED);

                    if (self._aborted) return;

                    // LOADING
                    self._setReadyState(LOADING);

                    if (self._aborted) return;

                    // Read the body
                    return response.text().then(function (text) {
                        if (self._aborted) return;

                        self.status     = response.status     || 0;
                        self.statusText = response.statusText || '';
                        self.responseText = text;

                        // Populate .response according to responseType
                        var rt = self.responseType;
                        if (!rt || rt === 'text') {
                            self.response = text;
                        } else if (rt === 'json') {
                            try {
                                self.response = JSON.parse(text);
                            } catch (_e) {
                                self.response = null;
                            }
                        } else {
                            // 'blob', 'arraybuffer', 'document' – best-effort text fallback
                            self.response = text;
                        }

                        var loaded = text.length;

                        // DONE
                        self._setReadyState(DONE);

                        self._fireEvent('progress', { loaded: loaded, total: loaded, lengthComputable: true });
                        self._fireEvent('load',     { loaded: loaded, total: loaded, lengthComputable: true });
                        self._fireEvent('loadend',  { loaded: loaded, total: loaded, lengthComputable: false });
                    });
                })
                .catch(function (err) {
                    if (self._aborted) return;

                    self.status     = 0;
                    self.statusText = '';
                    self._setReadyState(DONE);

                    self._fireEvent('error',   { error: err });
                    self._fireEvent('loadend', { loaded: 0, total: 0 });
                });
        } catch (e) {
            // Synchronous throw from fetch() itself
            this.status     = 0;
            this.statusText = '';
            this._setReadyState(DONE);
            this._fireEvent('error',   { error: e });
            this._fireEvent('loadend', { loaded: 0, total: 0 });
        }
    };

    // -----------------------------------------------------------------------
    // Expose on the global object
    // -----------------------------------------------------------------------
    globalThis.XMLHttpRequest = XMLHttpRequest;
})();
"#;

    runtime.execute(script, false).map_err(|e| {
        eprintln!("[JS] Warning: Failed to set up XMLHttpRequest: {}", e);
        e
    })?;

    Ok(())
}

