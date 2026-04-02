use std::cell::RefCell;
use std::collections::HashMap;

use mozjs::context::{JSContext, RawJSContext};
use mozjs::jsapi::{CallArgs, ExceptionStackBehavior, HandleValueArray, JSObject, JSPROP_ENUMERATE};
use mozjs::jsval::{BooleanValue, JSVal, NullValue, ObjectValue, UndefinedValue};
use mozjs::rooted;
use mozjs::rust::wrappers2::{CurrentGlobalOrNull, JS_CallFunctionValue, JS_ClearPendingException, JS_DefineProperty, JS_GetProperty, JS_SetElement, JS_SetPendingException};
use mozjs::rust::ValueArray;

use crate::js::bindings::dom_bindings::DOM_REF;
use crate::js::bindings::element_bindings::create_js_node_wrapper_by_id;
use crate::js::helpers::{create_empty_array, create_js_string, define_function, get_node_id_from_this, get_node_id_from_value, js_value_to_string, set_int_property, set_string_property, ToSafeCx};
use crate::js::jsapi::promise::PersistentRooted;
use crate::js::{JsResult, JsRuntime};

const OBSERVER_ID_PROP: &str = "__stokesMutationObserverId";

#[derive(Clone, Copy, PartialEq, Eq)]
enum MutationKind {
    ChildList,
    Attributes,
    CharacterData,
}

#[derive(Clone)]
struct MutationRecordData {
    kind: MutationKind,
    target_id: usize,
    added_nodes: Vec<usize>,
    removed_nodes: Vec<usize>,
    previous_sibling: Option<usize>,
    next_sibling: Option<usize>,
    attribute_name: Option<String>,
    old_value: Option<String>,
}

#[derive(Clone)]
struct ObserveOptions {
    child_list: bool,
    attributes: bool,
    character_data: bool,
    subtree: bool,
    attribute_old_value: bool,
    character_data_old_value: bool,
    attribute_filter: Option<Vec<String>>,
}

struct ObserverRegistration {
    target_id: usize,
    options: ObserveOptions,
}

struct MutationObserverEntry {
    observer_obj: PersistentRooted,
    callback_obj: PersistentRooted,
    registrations: Vec<ObserverRegistration>,
    records: Vec<MutationRecordData>,
}

#[derive(Default)]
struct MutationObserverState {
    next_id: i32,
    delivery_pending: bool,
    observers: HashMap<i32, MutationObserverEntry>,
}

thread_local! {
    static MUTATION_OBSERVER_STATE: RefCell<MutationObserverState> = RefCell::new(MutationObserverState::default());
}

pub fn clear_mutation_observer_state() {
    MUTATION_OBSERVER_STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.next_id = 0;
        state.delivery_pending = false;
        state.observers.clear();
    });
}

pub fn setup_mutation_observer(runtime: &mut JsRuntime) -> JsResult<()> {
    runtime.do_with_jsapi(|cx, global| unsafe {
        define_function(cx, global.get(), "MutationObserver", Some(mutation_observer_constructor), 1)?;
        define_function(cx, global.get(), "MutationRecord", Some(mutation_record_constructor), 0)?;

        let raw_cx = cx.raw_cx();
        rooted!(in(raw_cx) let global_rooted = global.get());

        rooted!(in(raw_cx) let mut mo_ctor = UndefinedValue());
        let mo_name = std::ffi::CString::new("MutationObserver").unwrap();
        if !JS_GetProperty(cx, global_rooted.handle().into(), mo_name.as_ptr(), mo_ctor.handle_mut().into()) || !mo_ctor.get().is_object() {
            return Err("Failed to resolve MutationObserver constructor".to_string());
        }

        rooted!(in(raw_cx) let mo_ctor_obj = mo_ctor.get().to_object());
        rooted!(in(raw_cx) let mut mo_proto = UndefinedValue());
        let proto_name = std::ffi::CString::new("prototype").unwrap();
        if !JS_GetProperty(cx, mo_ctor_obj.handle().into(), proto_name.as_ptr(), mo_proto.handle_mut().into()) || !mo_proto.get().is_object() {
            return Err("Failed to resolve MutationObserver.prototype".to_string());
        }

        define_function(cx, mo_proto.get().to_object(), "observe", Some(mutation_observer_observe), 2)?;
        define_function(cx, mo_proto.get().to_object(), "disconnect", Some(mutation_observer_disconnect), 0)?;
        define_function(cx, mo_proto.get().to_object(), "takeRecords", Some(mutation_observer_take_records), 0)?;

        rooted!(in(raw_cx) let mo_ctor_val = ObjectValue(mo_ctor_obj.get()));
        let webkit_name = std::ffi::CString::new("WebKitMutationObserver").unwrap();
        JS_DefineProperty(
            cx,
            global_rooted.handle().into(),
            webkit_name.as_ptr(),
            mo_ctor_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );

        Ok(())
    })
}

pub(crate) fn queue_child_list_mutation(
    target_id: usize,
    added_nodes: Vec<usize>,
    removed_nodes: Vec<usize>,
    previous_sibling: Option<usize>,
    next_sibling: Option<usize>,
) {
    queue_mutation(MutationRecordData {
        kind: MutationKind::ChildList,
        target_id,
        added_nodes,
        removed_nodes,
        previous_sibling,
        next_sibling,
        attribute_name: None,
        old_value: None,
    });
}

pub(crate) fn queue_attribute_mutation(target_id: usize, attribute_name: String, old_value: Option<String>) {
    queue_mutation(MutationRecordData {
        kind: MutationKind::Attributes,
        target_id,
        added_nodes: Vec::new(),
        removed_nodes: Vec::new(),
        previous_sibling: None,
        next_sibling: None,
        attribute_name: Some(attribute_name),
        old_value,
    });
}

pub(crate) fn queue_character_data_mutation(target_id: usize, old_value: Option<String>) {
    queue_mutation(MutationRecordData {
        kind: MutationKind::CharacterData,
        target_id,
        added_nodes: Vec::new(),
        removed_nodes: Vec::new(),
        previous_sibling: None,
        next_sibling: None,
        attribute_name: None,
        old_value,
    });
}

fn queue_mutation(base_record: MutationRecordData) {
    MUTATION_OBSERVER_STATE.with(|state| {
        let mut state = state.borrow_mut();
        let mut any_queued = false;

        for observer in state.observers.values_mut() {
            for registration in &observer.registrations {
                if !registration_matches_record(registration, &base_record) {
                    continue;
                }
                observer.records.push(clone_record_for_observer(&base_record, &registration.options));
                any_queued = true;
                break;
            }
        }

        if any_queued {
            state.delivery_pending = true;
        }
    });
}

pub(crate) unsafe fn deliver_pending_mutation_observers(cx: &mut JSContext) {
    let should_deliver = MUTATION_OBSERVER_STATE.with(|state| {
        let mut state = state.borrow_mut();
        if !state.delivery_pending {
            return false;
        }
        state.delivery_pending = false;
        true
    });

    if !should_deliver {
        return;
    }

    let mut observer_ids = MUTATION_OBSERVER_STATE.with(|state| {
        state.borrow().observers.keys().copied().collect::<Vec<i32>>()
    });
    observer_ids.sort_unstable();

    for observer_id in observer_ids {
        let payload = MUTATION_OBSERVER_STATE.with(|state| {
            let mut state = state.borrow_mut();
            let observer = state.observers.get_mut(&observer_id)?;
            if observer.records.is_empty() {
                return None;
            }
            let records = std::mem::take(&mut observer.records);
            Some((observer.observer_obj.get(), observer.callback_obj.get(), records))
        });

        let Some((observer_obj, callback_obj, records)) = payload else {
            continue;
        };

        let raw_cx = cx.raw_cx();
        rooted!(in(raw_cx) let observer_rooted = observer_obj);
        rooted!(in(raw_cx) let callback_rooted = callback_obj);

        let records_array = mutation_records_to_js_array(cx, &records);
        if records_array.is_null() {
            continue;
        }

        rooted!(in(raw_cx) let records_val = ObjectValue(records_array));
        rooted!(in(raw_cx) let observer_val = ObjectValue(observer_obj));
        rooted!(in(raw_cx) let call_args = ValueArray::<2usize>::new([records_val.get(), observer_val.get()]));
        rooted!(in(raw_cx) let mut rval = UndefinedValue());
        if !JS_CallFunctionValue(
            cx,
            observer_rooted.handle().into(),
            callback_rooted.handle().into(),
            &HandleValueArray::from(&call_args),
            rval.handle_mut().into(),
        ) {
            JS_ClearPendingException(cx);
        }
    }
}

fn registration_matches_record(registration: &ObserverRegistration, record: &MutationRecordData) -> bool {
    if !matches_observed_target(registration, record.target_id) {
        return false;
    }

    match record.kind {
        MutationKind::ChildList => registration.options.child_list,
        MutationKind::CharacterData => registration.options.character_data,
        MutationKind::Attributes => {
            if !registration.options.attributes {
                return false;
            }

            if let Some(filter) = &registration.options.attribute_filter {
                if filter.is_empty() {
                    return true;
                }
                let Some(name) = &record.attribute_name else {
                    return false;
                };
                return filter.iter().any(|entry| entry == name);
            }

            true
        }
    }
}

fn matches_observed_target(registration: &ObserverRegistration, target_id: usize) -> bool {
    if registration.target_id == target_id {
        return true;
    }
    if !registration.options.subtree {
        return false;
    }

    DOM_REF.with(|dom_ref| {
        let Some(dom_ptr) = *dom_ref.borrow() else {
            return false;
        };
        let dom = unsafe { &*dom_ptr };
        let mut current = Some(target_id);
        while let Some(node_id) = current {
            if node_id == registration.target_id {
                return true;
            }
            current = dom.parent_id(node_id);
        }
        false
    })
}

fn clone_record_for_observer(base: &MutationRecordData, options: &ObserveOptions) -> MutationRecordData {
    let old_value = match base.kind {
        MutationKind::Attributes if options.attribute_old_value => base.old_value.clone(),
        MutationKind::CharacterData if options.character_data_old_value => base.old_value.clone(),
        _ => None,
    };

    MutationRecordData {
        kind: base.kind,
        target_id: base.target_id,
        added_nodes: base.added_nodes.clone(),
        removed_nodes: base.removed_nodes.clone(),
        previous_sibling: base.previous_sibling,
        next_sibling: base.next_sibling,
        attribute_name: base.attribute_name.clone(),
        old_value,
    }
}

unsafe extern "C" fn mutation_observer_constructor(raw_cx: *mut RawJSContext, argc: u32, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let cx = &mut raw_cx.to_safe_cx();
    let raw_cx = cx.raw_cx();

    if argc < 1 || !args.get(0).is_object() {
        return throw_type_error(cx, "Failed to construct 'MutationObserver': callback must be a function.");
    }

    if !args.thisv().is_object() || args.thisv().is_null() {
        return throw_type_error(cx, "Failed to construct 'MutationObserver': callback must be a function.");
    }

    let callback_obj = args.get(0).to_object();
    let observer_obj = args.thisv().to_object();

    let mut observer_root = PersistentRooted::new();
    observer_root.init(cx, observer_obj);
    let mut callback_root = PersistentRooted::new();
    callback_root.init(cx, callback_obj);

    let observer_id = MUTATION_OBSERVER_STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.next_id += 1;
        let id = state.next_id;
        state.observers.insert(
            id,
            MutationObserverEntry {
                observer_obj: observer_root,
                callback_obj: callback_root,
                registrations: Vec::new(),
                records: Vec::new(),
            },
        );
        id
    });

    if set_int_property(cx, observer_obj, OBSERVER_ID_PROP, observer_id).is_err() {
        return throw_type_error(cx, "Failed to construct 'MutationObserver': internal setup failed.");
    }

    rooted!(in(raw_cx) let observer_val = ObjectValue(observer_obj));
    args.rval().set(*observer_val);
    true
}

unsafe extern "C" fn mutation_record_constructor(raw_cx: *mut RawJSContext, _argc: u32, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, 0);
    let cx = &mut raw_cx.to_safe_cx();
    let _ = args;
    throw_type_error(cx, "Illegal constructor")
}

unsafe extern "C" fn mutation_observer_observe(raw_cx: *mut RawJSContext, argc: u32, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let cx = &mut raw_cx.to_safe_cx();

    let Some(observer_id) = get_observer_id(cx, &args) else {
        return throw_type_error(cx, "Failed to execute 'observe' on 'MutationObserver': invalid observer.");
    };

    if argc < 1 {
        return throw_type_error(cx, "Failed to execute 'observe' on 'MutationObserver': parameter 1 is not of type 'Node'.");
    }
    let Some(target_id) = get_node_id_from_value(cx, *args.get(0)) else {
        return throw_type_error(cx, "Failed to execute 'observe' on 'MutationObserver': parameter 1 is not of type 'Node'.");
    };

    if argc < 2 || !args.get(1).is_object() || args.get(1).is_null() {
        return throw_type_error(cx, "Failed to execute 'observe' on 'MutationObserver': parameter 2 is not an object.");
    }

    let options_obj = args.get(1).to_object();
    let options = match normalize_observe_options(cx, options_obj) {
        Ok(options) => options,
        Err(message) => return throw_type_error(cx, &message),
    };

    MUTATION_OBSERVER_STATE.with(|state| {
        let mut state = state.borrow_mut();
        if let Some(observer) = state.observers.get_mut(&observer_id) {
            if let Some(existing) = observer.registrations.iter_mut().find(|registration| registration.target_id == target_id) {
                existing.options = options;
            } else {
                observer.registrations.push(ObserverRegistration { target_id, options });
            }
        }
    });

    args.rval().set(UndefinedValue());
    true
}

unsafe extern "C" fn mutation_observer_disconnect(raw_cx: *mut RawJSContext, argc: u32, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let cx = &mut raw_cx.to_safe_cx();

    if let Some(observer_id) = get_observer_id(cx, &args) {
        MUTATION_OBSERVER_STATE.with(|state| {
            let mut state = state.borrow_mut();
            if let Some(observer) = state.observers.get_mut(&observer_id) {
                observer.registrations.clear();
                observer.records.clear();
            }
        });
    }

    args.rval().set(UndefinedValue());
    true
}

unsafe extern "C" fn mutation_observer_take_records(raw_cx: *mut RawJSContext, argc: u32, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let cx = &mut raw_cx.to_safe_cx();

    let records = if let Some(observer_id) = get_observer_id(cx, &args) {
        MUTATION_OBSERVER_STATE.with(|state| {
            let mut state = state.borrow_mut();
            state
                .observers
                .get_mut(&observer_id)
                .map(|observer| std::mem::take(&mut observer.records))
                .unwrap_or_default()
        })
    } else {
        Vec::new()
    };

    let array = mutation_records_to_js_array(cx, &records);
    if array.is_null() {
        args.rval().set(UndefinedValue());
    } else {
        args.rval().set(ObjectValue(array));
    }
    true
}

unsafe fn mutation_records_to_js_array(cx: &mut JSContext, records: &[MutationRecordData]) -> *mut JSObject {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let records_array = create_empty_array(cx));
    if records_array.get().is_null() {
        return std::ptr::null_mut();
    }

    for (idx, record) in records.iter().enumerate() {
        let record_obj = mutation_record_to_js_object(cx, record);
        if record_obj.is_null() {
            continue;
        }
        rooted!(in(raw_cx) let record_val = ObjectValue(record_obj));
        rooted!(in(raw_cx) let array_obj = records_array.get());
        JS_SetElement(cx, array_obj.handle().into(), idx as u32, record_val.handle().into());
    }

    records_array.get()
}

unsafe fn mutation_record_to_js_object(cx: &mut JSContext, record: &MutationRecordData) -> *mut JSObject {
    use mozjs::rust::wrappers2::JS_NewPlainObject;

    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let record_obj = JS_NewPlainObject(cx));
    if record_obj.get().is_null() {
        return std::ptr::null_mut();
    }

    let record_type = match record.kind {
        MutationKind::ChildList => "childList",
        MutationKind::Attributes => "attributes",
        MutationKind::CharacterData => "characterData",
    };
    let _ = set_string_property(cx, record_obj.get(), "type", record_type);

    if let Some(target_val) = create_js_node_wrapper_by_id(cx, record.target_id) {
        rooted!(in(raw_cx) let target_rooted = target_val);
        let key = std::ffi::CString::new("target").unwrap();
        JS_DefineProperty(cx, record_obj.handle().into(), key.as_ptr(), target_rooted.handle().into(), JSPROP_ENUMERATE as u32);
    } else {
        rooted!(in(raw_cx) let null_val = NullValue());
        let key = std::ffi::CString::new("target").unwrap();
        JS_DefineProperty(cx, record_obj.handle().into(), key.as_ptr(), null_val.handle().into(), JSPROP_ENUMERATE as u32);
    }

    define_node_list_property(cx, record_obj.get(), "addedNodes", &record.added_nodes);
    define_node_list_property(cx, record_obj.get(), "removedNodes", &record.removed_nodes);
    define_optional_node_property(cx, record_obj.get(), "previousSibling", record.previous_sibling);
    define_optional_node_property(cx, record_obj.get(), "nextSibling", record.next_sibling);
    define_optional_string_property(cx, record_obj.get(), "attributeName", record.attribute_name.as_deref());
    define_optional_string_property(cx, record_obj.get(), "attributeNamespace", None);
    define_optional_string_property(cx, record_obj.get(), "oldValue", record.old_value.as_deref());

    record_obj.get()
}

unsafe fn define_node_list_property(cx: &mut JSContext, obj: *mut JSObject, name: &str, node_ids: &[usize]) {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let array = create_empty_array(cx));
    if array.get().is_null() {
        return;
    }

    for (idx, node_id) in node_ids.iter().copied().enumerate() {
        if let Some(node_val) = create_js_node_wrapper_by_id(cx, node_id) {
            rooted!(in(raw_cx) let node_rooted = node_val);
            rooted!(in(raw_cx) let array_obj = array.get());
            JS_SetElement(cx, array_obj.handle().into(), idx as u32, node_rooted.handle().into());
        }
    }

    let key = std::ffi::CString::new(name).unwrap();
    rooted!(in(raw_cx) let arr_val = ObjectValue(array.get()));
    rooted!(in(raw_cx) let obj_rooted = obj);
    JS_DefineProperty(cx, obj_rooted.handle().into(), key.as_ptr(), arr_val.handle().into(), JSPROP_ENUMERATE as u32);
}

unsafe fn define_optional_node_property(cx: &mut JSContext, obj: *mut JSObject, name: &str, node_id: Option<usize>) {
    let raw_cx = cx.raw_cx();
    let key = std::ffi::CString::new(name).unwrap();
    rooted!(in(raw_cx) let obj_rooted = obj);

    if let Some(node_id) = node_id {
        if let Some(node_val) = create_js_node_wrapper_by_id(cx, node_id) {
            rooted!(in(raw_cx) let node_rooted = node_val);
            JS_DefineProperty(cx, obj_rooted.handle().into(), key.as_ptr(), node_rooted.handle().into(), JSPROP_ENUMERATE as u32);
            return;
        }
    }

    rooted!(in(raw_cx) let null_val = NullValue());
    JS_DefineProperty(cx, obj_rooted.handle().into(), key.as_ptr(), null_val.handle().into(), JSPROP_ENUMERATE as u32);
}

unsafe fn define_optional_string_property(cx: &mut JSContext, obj: *mut JSObject, name: &str, value: Option<&str>) {
    let raw_cx = cx.raw_cx();
    let key = std::ffi::CString::new(name).unwrap();
    rooted!(in(raw_cx) let obj_rooted = obj);
    if let Some(value) = value {
        rooted!(in(raw_cx) let str_val = create_js_string(cx, value));
        JS_DefineProperty(cx, obj_rooted.handle().into(), key.as_ptr(), str_val.handle().into(), JSPROP_ENUMERATE as u32);
    } else {
        rooted!(in(raw_cx) let null_val = NullValue());
        JS_DefineProperty(cx, obj_rooted.handle().into(), key.as_ptr(), null_val.handle().into(), JSPROP_ENUMERATE as u32);
    }
}

unsafe fn normalize_observe_options(cx: &mut JSContext, options_obj: *mut JSObject) -> Result<ObserveOptions, String> {
    let (has_attributes, attributes_raw) = get_optional_property(cx, options_obj, "attributes");
    let (has_character_data, character_data_raw) = get_optional_property(cx, options_obj, "characterData");
    let (_has_child_list, child_list_raw) = get_optional_property(cx, options_obj, "childList");
    let (_has_subtree, subtree_raw) = get_optional_property(cx, options_obj, "subtree");
    let (_has_attr_old, attr_old_raw) = get_optional_property(cx, options_obj, "attributeOldValue");
    let (_has_char_old, char_old_raw) = get_optional_property(cx, options_obj, "characterDataOldValue");
    let (has_attr_filter, attr_filter_raw) = get_optional_property(cx, options_obj, "attributeFilter");

    let child_list = js_value_to_boolish(child_list_raw);
    let subtree = js_value_to_boolish(subtree_raw);
    let attribute_old_value = js_value_to_boolish(attr_old_raw);
    let character_data_old_value = js_value_to_boolish(char_old_raw);

    let attributes = if has_attributes {
        js_value_to_boolish(attributes_raw)
    } else {
        attribute_old_value || has_attr_filter
    };

    let character_data = if has_character_data {
        js_value_to_boolish(character_data_raw)
    } else {
        character_data_old_value
    };

    let attribute_filter = if has_attr_filter && !attr_filter_raw.is_null() {
        if !attr_filter_raw.is_object() {
            return Err("Failed to execute 'observe' on 'MutationObserver': 'attributeFilter' must be an array.".to_string());
        }
        Some(js_array_like_to_strings(cx, attr_filter_raw.to_object()))
    } else {
        None
    };

    if !child_list && !attributes && !character_data {
        return Err("Failed to execute 'observe' on 'MutationObserver': at least one of childList, attributes, or characterData must be true.".to_string());
    }
    if attribute_old_value && !attributes {
        return Err("Failed to execute 'observe' on 'MutationObserver': attributeOldValue requires attributes to be true.".to_string());
    }
    if character_data_old_value && !character_data {
        return Err("Failed to execute 'observe' on 'MutationObserver': characterDataOldValue requires characterData to be true.".to_string());
    }

    Ok(ObserveOptions {
        child_list,
        attributes,
        character_data,
        subtree,
        attribute_old_value,
        character_data_old_value,
        attribute_filter,
    })
}

unsafe fn get_optional_property(cx: &mut JSContext, obj: *mut JSObject, name: &str) -> (bool, JSVal) {
    let raw_cx = cx.raw_cx();
    let key = std::ffi::CString::new(name).unwrap();
    rooted!(in(raw_cx) let obj_rooted = obj);
    rooted!(in(raw_cx) let mut out = UndefinedValue());
    if !JS_GetProperty(cx, obj_rooted.handle().into(), key.as_ptr(), out.handle_mut().into()) {
        return (false, UndefinedValue());
    }
    (!out.get().is_undefined(), out.get())
}

unsafe fn js_array_like_to_strings(cx: &mut JSContext, arr_obj: *mut JSObject) -> Vec<String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let arr_rooted = arr_obj);
    rooted!(in(raw_cx) let mut len_val = UndefinedValue());
    let length_name = std::ffi::CString::new("length").unwrap();
    if !JS_GetProperty(cx, arr_rooted.handle().into(), length_name.as_ptr(), len_val.handle_mut().into()) {
        return Vec::new();
    }

    let len = if len_val.get().is_int32() {
        len_val.get().to_int32().max(0) as usize
    } else if len_val.get().is_double() {
        len_val.get().to_double().max(0.0) as usize
    } else {
        0
    };

    let mut out = Vec::with_capacity(len);
    for idx in 0..len {
        rooted!(in(raw_cx) let mut entry = UndefinedValue());
        let key = std::ffi::CString::new(idx.to_string()).unwrap();
        if JS_GetProperty(cx, arr_rooted.handle().into(), key.as_ptr(), entry.handle_mut().into()) {
            out.push(js_value_to_string(cx, entry.get()));
        }
    }
    out
}

fn js_value_to_boolish(value: JSVal) -> bool {
    if value.is_undefined() || value.is_null() {
        return false;
    }
    if value.is_boolean() {
        return value.to_boolean();
    }
    if value.is_int32() {
        return value.to_int32() != 0;
    }
    if value.is_double() {
        let f = value.to_double();
        return f != 0.0 && !f.is_nan();
    }
    if value.is_string() {
        return true;
    }
    value.is_object()
}

unsafe fn get_observer_id(cx: &mut JSContext, args: &CallArgs) -> Option<i32> {
    let raw_cx = cx.raw_cx();
    if !args.thisv().is_object() || args.thisv().is_null() {
        return None;
    }

    let this_obj = args.thisv().to_object();
    rooted!(in(raw_cx) let this_rooted = this_obj);
    rooted!(in(raw_cx) let mut id_val = UndefinedValue());
    let key = std::ffi::CString::new(OBSERVER_ID_PROP).unwrap();
    if !JS_GetProperty(cx, this_rooted.handle().into(), key.as_ptr(), id_val.handle_mut().into()) {
        return None;
    }

    if id_val.get().is_int32() {
        Some(id_val.get().to_int32())
    } else if id_val.get().is_double() {
        Some(id_val.get().to_double() as i32)
    } else {
        None
    }
}

unsafe fn throw_type_error(cx: &mut JSContext, message: &str) -> bool {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let global = CurrentGlobalOrNull(cx));
    if global.get().is_null() {
        return false;
    }

    rooted!(in(raw_cx) let mut ctor_val = UndefinedValue());
    let type_error_name = std::ffi::CString::new("TypeError").unwrap();
    if !JS_GetProperty(cx, global.handle().into(), type_error_name.as_ptr(), ctor_val.handle_mut().into()) || !ctor_val.get().is_object() {
        return false;
    }

    rooted!(in(raw_cx) let message_val = create_js_string(cx, message));
    rooted!(in(raw_cx) let args = ValueArray::<1usize>::new([message_val.get()]));
    rooted!(in(raw_cx) let mut err_obj = UndefinedValue());
    if !JS_CallFunctionValue(
        cx,
        global.handle().into(),
        ctor_val.handle().into(),
        &HandleValueArray::from(&args),
        err_obj.handle_mut().into(),
    ) {
        return false;
    }

    JS_SetPendingException(
        cx,
        err_obj.handle().into(),
        ExceptionStackBehavior::DoNotCapture,
    );
    false
}

