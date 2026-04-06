use crate::js::JsRuntime;
use tracing::warn;

pub(crate) fn setup_document_implementation_deferred(runtime: &mut JsRuntime) -> Result<(), String> {
    let script = r#"
        (function() {
            const root = typeof globalThis !== 'undefined' ? globalThis : window;
            const doc = root.document;
            if (!doc) {
                return;
            }

            function defineValue(target, key, value) {
                Object.defineProperty(target, key, {
                    value,
                    writable: true,
                    configurable: true,
                    enumerable: true,
                });
            }

            function makeDocType(qualifiedName, publicId, systemId) {
                return {
                    name: String(qualifiedName == null ? '' : qualifiedName),
                    nodeName: String(qualifiedName == null ? '' : qualifiedName),
                    publicId: String(publicId == null ? '' : publicId),
                    systemId: String(systemId == null ? '' : systemId),
                    nodeType: 10,
                    ownerDocument: null,
                };
            }

            function toArrayLikeList(single) {
                const list = [single];
                list.item = function(index) {
                    return this[index] || null;
                };
                return list;
            }

            function makeDetachedHtmlDocument(title) {
                const html = doc.createElement('html');
                const head = doc.createElement('head');
                const body = doc.createElement('body');
                const titleElement = doc.createElement('title');
                let titleValue = String(title == null ? '' : title);
                titleElement.textContent = titleValue;
                head.appendChild(titleElement);
                html.appendChild(head);
                html.appendChild(body);

                const detached = {
                    nodeType: 9,
                    nodeName: '#document',
                    compatMode: 'CSS1Compat',
                    characterSet: 'UTF-8',
                    charset: 'UTF-8',
                    inputEncoding: 'UTF-8',
                    readyState: 'complete',
                    URL: 'about:blank',
                    documentURI: 'about:blank',
                    baseURI: 'about:blank',
                    defaultView: null,
                    doctype: null,
                };

                defineValue(detached, 'documentElement', html);
                defineValue(detached, 'head', head);
                defineValue(detached, 'body', body);

                Object.defineProperty(detached, 'title', {
                    get: function() {
                        return titleValue;
                    },
                    set: function(value) {
                        titleValue = String(value == null ? '' : value);
                        titleElement.textContent = titleValue;
                    },
                    configurable: true,
                    enumerable: true,
                });

                detached.createElement = doc.createElement.bind(doc);
                detached.createElementNS = doc.createElementNS.bind(doc);
                detached.createTextNode = doc.createTextNode.bind(doc);
                detached.createDocumentFragment = doc.createDocumentFragment.bind(doc);
                detached.getElementById = function(id) {
                    if (!id) {
                        return null;
                    }
                    if (typeof html.querySelector === 'function') {
                        return html.querySelector('#' + String(id));
                    }
                    return null;
                };
                detached.getElementsByTagName = function(tagName) {
                    if (html && typeof html.getElementsByTagName === 'function') {
                        return html.getElementsByTagName(String(tagName == null ? '' : tagName));
                    }
                    return [];
                };
                detached.querySelector = function(selector) {
                    return typeof html.querySelector === 'function' ? html.querySelector(selector) : null;
                };
                detached.querySelectorAll = function(selector) {
                    return typeof html.querySelectorAll === 'function' ? html.querySelectorAll(selector) : [];
                };
                detached.appendChild = function(node) {
                    return body.appendChild(node);
                };
                detached.removeChild = function(node) {
                    return body.removeChild(node);
                };
                detached.hasChildNodes = function() {
                    return !!html.hasChildNodes && html.hasChildNodes();
                };

                detached.getElementsByTagName('html').item = function(index) {
                    return index === 0 ? html : null;
                };
                detached.childNodes = toArrayLikeList(html);

                return detached;
            }

            function makeDetachedXmlDocument(namespace, qualifiedName, doctype) {
                const detached = {
                    nodeType: 9,
                    nodeName: '#document',
                    URL: 'about:blank',
                    documentURI: 'about:blank',
                    baseURI: 'about:blank',
                    defaultView: null,
                    doctype: doctype || null,
                };

                let rootElement = null;
                const qn = String(qualifiedName == null ? '' : qualifiedName);
                if (qn) {
                    if (namespace == null || namespace === '') {
                        rootElement = doc.createElement(qn);
                    } else {
                        rootElement = doc.createElementNS(String(namespace), qn);
                    }
                }

                defineValue(detached, 'documentElement', rootElement);
                detached.createElement = doc.createElement.bind(doc);
                detached.createElementNS = doc.createElementNS.bind(doc);
                detached.createTextNode = doc.createTextNode.bind(doc);
                detached.createDocumentFragment = doc.createDocumentFragment.bind(doc);
                detached.appendChild = function(node) {
                    detached.documentElement = node;
                    return node;
                };
                detached.removeChild = function(node) {
                    if (detached.documentElement === node) {
                        detached.documentElement = null;
                    }
                    return node;
                };
                detached.childNodes = rootElement ? toArrayLikeList(rootElement) : [];

                return detached;
            }

            function DOMImplementation() {
                throw new TypeError('Illegal constructor');
            }

            DOMImplementation.prototype.hasFeature = function() {
                return true;
            };

            DOMImplementation.prototype.createDocumentType = function(qualifiedName, publicId, systemId) {
                return makeDocType(qualifiedName, publicId, systemId);
            };

            DOMImplementation.prototype.createDocument = function(namespace, qualifiedName, doctype) {
                const detached = makeDetachedXmlDocument(namespace, qualifiedName, doctype || null);
                detached.implementation = this;
                if (detached.doctype && typeof detached.doctype === 'object') {
                    detached.doctype.ownerDocument = detached;
                }
                return detached;
            };

            DOMImplementation.prototype.createHTMLDocument = function(title) {
                const detached = makeDetachedHtmlDocument(title);
                detached.implementation = this;
                return detached;
            };

            Object.defineProperty(DOMImplementation.prototype, Symbol.toStringTag, {
                value: 'DOMImplementation',
                configurable: true,
            });

            const impl = Object.create(DOMImplementation.prototype);

            Object.defineProperty(doc, 'implementation', {
                value: impl,
                writable: false,
                configurable: true,
                enumerable: true,
            });

            Object.defineProperty(root, 'DOMImplementation', {
                value: DOMImplementation,
                writable: true,
                configurable: true,
                enumerable: true,
            });
        })();
    "#;

    runtime.execute(script, false).map_err(|e| {
        warn!("[JS] Failed to set up document.implementation: {}", e);
        e
    })?;

    Ok(())
}

