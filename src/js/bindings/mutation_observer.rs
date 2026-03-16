use crate::js::{JsResult, JsRuntime};

/// Install a MutationObserver polyfill and mutation instrumentation hooks.
///
/// The engine's DOM wrappers are lightweight plain objects, so we patch each created
/// element wrapper through `__stokesPatchMutationObserverNode` (called from Rust).
pub fn setup_mutation_observer(runtime: &mut JsRuntime) -> JsResult<()> {
    let script = r#"
        (function() {
            const root = typeof globalThis !== 'undefined' ? globalThis : window;
            if (!root || root.MutationObserver) {
                return;
            }

            const observerState = {
                observers: new Set(),
                scheduled: false,
            };

            const PATCH_MARK = '__stokesMoPatched';

            function isNodeLike(value) {
                return !!value && typeof value === 'object';
            }

            function toNodeList(items) {
                if (!items || !items.length) {
                    return [];
                }
                const out = [];
                for (let i = 0; i < items.length; i += 1) {
                    const item = items[i];
                    if (isNodeLike(item)) {
                        out.push(item);
                    }
                }
                return out;
            }

            function normalizeOptions(options) {
                if (!options || typeof options !== 'object') {
                    throw new TypeError("Failed to execute 'observe' on 'MutationObserver': parameter 2 is not an object.");
                }

                const hasAttributes = Object.prototype.hasOwnProperty.call(options, 'attributes');
                const hasCharacterData = Object.prototype.hasOwnProperty.call(options, 'characterData');
                const childList = !!options.childList;
                const attributes = hasAttributes
                    ? !!options.attributes
                    : (options.attributeOldValue === true || Array.isArray(options.attributeFilter));
                const characterData = hasCharacterData
                    ? !!options.characterData
                    : (options.characterDataOldValue === true);
                const subtree = !!options.subtree;
                const attributeOldValue = !!options.attributeOldValue;
                const characterDataOldValue = !!options.characterDataOldValue;

                let attributeFilter = null;
                if (options.attributeFilter != null) {
                    if (!Array.isArray(options.attributeFilter)) {
                        throw new TypeError("Failed to execute 'observe' on 'MutationObserver': 'attributeFilter' must be an array.");
                    }
                    attributeFilter = options.attributeFilter.map((name) => String(name));
                }

                if (!childList && !attributes && !characterData) {
                    throw new TypeError("Failed to execute 'observe' on 'MutationObserver': at least one of childList, attributes, or characterData must be true.");
                }

                if (options.attributeOldValue === true && !attributes) {
                    throw new TypeError("Failed to execute 'observe' on 'MutationObserver': attributeOldValue requires attributes to be true.");
                }

                if (options.characterDataOldValue === true && !characterData) {
                    throw new TypeError("Failed to execute 'observe' on 'MutationObserver': characterDataOldValue requires characterData to be true.");
                }

                return {
                    childList,
                    attributes,
                    characterData,
                    subtree,
                    attributeOldValue,
                    characterDataOldValue,
                    attributeFilter,
                };
            }

            function patchSubtree(target) {
                patchNode(target);
                if (!target || typeof target.querySelectorAll !== 'function') {
                    return;
                }
                try {
                    const descendants = target.querySelectorAll('*');
                    for (let i = 0; i < descendants.length; i += 1) {
                        patchNode(descendants[i]);
                    }
                } catch (_err) {}
            }

            function scheduleDelivery() {
                if (observerState.scheduled) {
                    return;
                }
                observerState.scheduled = true;
                Promise.resolve().then(flushObservers);
            }

            function flushObservers() {
                observerState.scheduled = false;
                observerState.observers.forEach((observer) => {
                    if (!observer._records.length) {
                        return;
                    }
                    const records = observer.takeRecords();
                    try {
                        observer._callback.call(observer, records, observer);
                    } catch (_err) {}
                });
            }

            function matchesObservedTarget(registration, targetNode) {
                if (registration.target === targetNode) {
                    return true;
                }
                if (!registration.options.subtree || !registration.target) {
                    return false;
                }
                if (typeof registration.target.contains === 'function') {
                    try {
                        return !!registration.target.contains(targetNode);
                    } catch (_err) {
                        return false;
                    }
                }
                return false;
            }

            function shouldReceiveRecord(registration, record) {
                const opts = registration.options;
                if (!matchesObservedTarget(registration, record.target)) {
                    return false;
                }

                if (record.type === 'attributes') {
                    if (!opts.attributes) {
                        return false;
                    }
                    if (opts.attributeFilter && opts.attributeFilter.length > 0) {
                        return opts.attributeFilter.indexOf(record.attributeName || '') >= 0;
                    }
                    return true;
                }

                if (record.type === 'characterData') {
                    return opts.characterData;
                }

                if (record.type === 'childList') {
                    return opts.childList;
                }

                return false;
            }

            function cloneRecordForObserver(record, registration) {
                const opts = registration.options;
                const out = Object.create(root.MutationRecord && root.MutationRecord.prototype ? root.MutationRecord.prototype : Object.prototype);

                out.type = record.type;
                out.target = record.target || null;
                out.addedNodes = toNodeList(record.addedNodes);
                out.removedNodes = toNodeList(record.removedNodes);
                out.previousSibling = record.previousSibling || null;
                out.nextSibling = record.nextSibling || null;
                out.attributeName = record.attributeName != null ? record.attributeName : null;
                out.attributeNamespace = null;

                if (record.type === 'attributes') {
                    out.oldValue = opts.attributeOldValue ? (record.oldValue != null ? String(record.oldValue) : null) : null;
                } else if (record.type === 'characterData') {
                    out.oldValue = opts.characterDataOldValue ? (record.oldValue != null ? String(record.oldValue) : null) : null;
                } else {
                    out.oldValue = null;
                }

                return out;
            }

            function enqueueRecord(record) {
                if (!record || !record.type || !record.target) {
                    return;
                }

                observerState.observers.forEach((observer) => {
                    for (let i = 0; i < observer._registrations.length; i += 1) {
                        const registration = observer._registrations[i];
                        if (!shouldReceiveRecord(registration, record)) {
                            continue;
                        }
                        observer._records.push(cloneRecordForObserver(record, registration));
                        scheduleDelivery();
                        break;
                    }
                });
            }

            class MutationObserver {
                constructor(callback) {
                    if (typeof callback !== 'function') {
                        throw new TypeError("Failed to construct 'MutationObserver': callback must be a function.");
                    }
                    this._callback = callback;
                    this._records = [];
                    this._registrations = [];
                    observerState.observers.add(this);
                }

                observe(target, options) {
                    if (!isNodeLike(target)) {
                        throw new TypeError("Failed to execute 'observe' on 'MutationObserver': parameter 1 is not of type 'Node'.");
                    }

                    const normalized = normalizeOptions(options);

                    for (let i = 0; i < this._registrations.length; i += 1) {
                        if (this._registrations[i].target === target) {
                            this._registrations[i].options = normalized;
                            patchSubtree(target);
                            return;
                        }
                    }

                    this._registrations.push({ target, options: normalized });
                    patchSubtree(target);
                }

                disconnect() {
                    this._registrations.length = 0;
                    this._records.length = 0;
                }

                takeRecords() {
                    const copy = this._records.slice();
                    this._records.length = 0;
                    return copy;
                }
            }

            function MutationRecord() {
                throw new TypeError('Illegal constructor');
            }

            root.MutationObserver = MutationObserver;
            root.WebKitMutationObserver = MutationObserver;
            root.MutationRecord = MutationRecord;

            function wrapMethod(node, name, buildRecord) {
                if (!node || typeof node[name] !== 'function') {
                    return;
                }
                const original = node[name];
                node[name] = function() {
                    const args = Array.prototype.slice.call(arguments);

                    let oldValue = null;
                    if (name === 'setAttribute' && args.length > 0) {
                        try {
                            oldValue = this.getAttribute(String(args[0]));
                        } catch (_err) {
                            oldValue = null;
                        }
                    }
                    if (name === 'removeAttribute' && args.length > 0) {
                        try {
                            oldValue = this.getAttribute(String(args[0]));
                        } catch (_err) {
                            oldValue = null;
                        }
                    }
                    if (name === '__setTextContent' && typeof this.__getTextContent === 'function') {
                        try {
                            oldValue = this.__getTextContent();
                        } catch (_err) {
                            oldValue = null;
                        }
                    }

                    const ret = original.apply(this, args);

                    const record = buildRecord(this, args, ret, oldValue);
                    if (record) {
                        if (record.type === 'childList' && record.addedNodes && record.addedNodes.length) {
                            for (let i = 0; i < record.addedNodes.length; i += 1) {
                                patchSubtree(record.addedNodes[i]);
                            }
                        }
                        enqueueRecord(record);
                    }

                    return ret;
                };
            }

            function makeChildListRecord(target, addedNodes, removedNodes, previousSibling, nextSibling) {
                return {
                    type: 'childList',
                    target,
                    addedNodes: toNodeList(addedNodes),
                    removedNodes: toNodeList(removedNodes),
                    previousSibling: previousSibling || null,
                    nextSibling: nextSibling || null,
                    oldValue: null,
                    attributeName: null,
                };
            }

            function patchNode(node) {
                if (!node) {
                    node = this;
                }
                if (!node || typeof node !== 'object') {
                    return node;
                }
                if (node[PATCH_MARK]) {
                    return node;
                }
                node[PATCH_MARK] = true;

                wrapMethod(node, 'appendChild', function(target, args) {
                    const child = args.length > 0 ? args[0] : null;
                    if (!isNodeLike(child)) {
                        return null;
                    }
                    return makeChildListRecord(target, [child], [], null, null);
                });

                wrapMethod(node, 'removeChild', function(target, args) {
                    const child = args.length > 0 ? args[0] : null;
                    if (!isNodeLike(child)) {
                        return null;
                    }
                    return makeChildListRecord(target, [], [child], null, null);
                });

                wrapMethod(node, 'insertBefore', function(target, args) {
                    const newNode = args.length > 0 ? args[0] : null;
                    const refNode = args.length > 1 ? args[1] : null;
                    if (!isNodeLike(newNode)) {
                        return null;
                    }
                    return makeChildListRecord(target, [newNode], [], null, isNodeLike(refNode) ? refNode : null);
                });

                wrapMethod(node, 'replaceChild', function(target, args) {
                    const newNode = args.length > 0 ? args[0] : null;
                    const oldNode = args.length > 1 ? args[1] : null;
                    if (!isNodeLike(newNode) || !isNodeLike(oldNode)) {
                        return null;
                    }
                    return makeChildListRecord(target, [newNode], [oldNode], null, null);
                });

                wrapMethod(node, 'remove', function(target) {
                    if (!target.parentNode) {
                        return null;
                    }
                    return makeChildListRecord(target.parentNode, [], [target], null, null);
                });

                wrapMethod(node, 'before', function(target, args) {
                    if (!target.parentNode) {
                        return null;
                    }
                    return makeChildListRecord(target.parentNode, toNodeList(args), [], null, target);
                });

                wrapMethod(node, 'after', function(target, args) {
                    if (!target.parentNode) {
                        return null;
                    }
                    return makeChildListRecord(target.parentNode, toNodeList(args), [], target, null);
                });

                wrapMethod(node, 'replaceWith', function(target, args) {
                    if (!target.parentNode) {
                        return null;
                    }
                    return makeChildListRecord(target.parentNode, toNodeList(args), [target], null, null);
                });

                wrapMethod(node, 'prepend', function(target, args) {
                    return makeChildListRecord(target, toNodeList(args), [], null, target.firstChild || null);
                });

                wrapMethod(node, 'append', function(target, args) {
                    return makeChildListRecord(target, toNodeList(args), [], target.lastChild || null, null);
                });

                wrapMethod(node, 'insertAdjacentElement', function(target, args, ret) {
                    const where = args.length > 0 ? String(args[0]).toLowerCase() : '';
                    const newNode = args.length > 1 ? args[1] : ret;
                    if (!isNodeLike(newNode)) {
                        return null;
                    }

                    if (where === 'beforebegin' || where === 'afterend') {
                        if (!target.parentNode) {
                            return null;
                        }
                        return makeChildListRecord(target.parentNode, [newNode], [], null, null);
                    }

                    return makeChildListRecord(target, [newNode], [], null, null);
                });

                wrapMethod(node, 'setAttribute', function(target, args, _ret, oldValue) {
                    const attributeName = args.length > 0 ? String(args[0]) : '';
                    return {
                        type: 'attributes',
                        target,
                        addedNodes: [],
                        removedNodes: [],
                        previousSibling: null,
                        nextSibling: null,
                        attributeName,
                        oldValue,
                    };
                });

                wrapMethod(node, 'removeAttribute', function(target, args, _ret, oldValue) {
                    const attributeName = args.length > 0 ? String(args[0]) : '';
                    return {
                        type: 'attributes',
                        target,
                        addedNodes: [],
                        removedNodes: [],
                        previousSibling: null,
                        nextSibling: null,
                        attributeName,
                        oldValue,
                    };
                });

                wrapMethod(node, '__setTextContent', function(target, _args, _ret, oldValue) {
                    return {
                        type: 'characterData',
                        target,
                        addedNodes: [],
                        removedNodes: [],
                        previousSibling: null,
                        nextSibling: null,
                        attributeName: null,
                        oldValue,
                    };
                });

                return node;
            }

            root.__stokesPatchMutationObserverNode = patchNode;
        })();
    "#;

    runtime.execute(script, false)
}

