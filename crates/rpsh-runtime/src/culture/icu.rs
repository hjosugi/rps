//! First-use binding to the system ICU collation API. All handles are owned
//! by one thread. A collator keeps its library alive until after ucol_close.
#![allow(
    unsafe_code,
    reason = "Issue #36 owns the small POSIX and ICU C ABI boundary"
)]

use super::{Casing, NormalizationForm, text::map_units};
use crate::PsString;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr::{self, NonNull};
use std::rc::Rc;

#[cfg_attr(target_os = "linux", link(name = "dl"))]
unsafe extern "C" {
    fn dlopen(name: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
}

struct Library(NonNull<c_void>);

impl Library {
    fn open(name: &str) -> Option<Self> {
        let name = CString::new(name).ok()?;
        // SAFETY: name is NUL-terminated. RTLD_LAZY is 1 on our Unix targets.
        NonNull::new(unsafe { dlopen(name.as_ptr(), 1) }).map(Self)
    }

    fn symbol(&self, name: &str) -> Option<NonNull<c_void>> {
        let name = CString::new(name).ok()?;
        // SAFETY: self owns the live handle and name is NUL-terminated.
        NonNull::new(unsafe { dlsym(self.0.as_ptr(), name.as_ptr()) })
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        // SAFETY: each successful dlopen creates exactly one Library.
        unsafe { dlclose(self.0.as_ptr()) };
    }
}

type Open = unsafe extern "C" fn(*const c_char, *mut i32) -> *mut c_void;
type Close = unsafe extern "C" fn(*mut c_void);
type Compare = unsafe extern "C" fn(*const c_void, *const u16, i32, *const u16, i32) -> i32;
type Rules = unsafe extern "C" fn(*const c_void, *mut i32) -> *const u16;
type OpenRules =
    unsafe extern "C" fn(*const u16, i32, i32, i32, *mut c_void, *mut i32) -> *mut c_void;
type OpenElements = unsafe extern "C" fn(*const c_void, *const u16, i32, *mut i32) -> *mut c_void;
type Next = unsafe extern "C" fn(*mut c_void, *mut i32) -> i32;
type Strength = unsafe extern "C" fn(*const c_void) -> i32;
type SortKey = unsafe extern "C" fn(*const c_void, *const u16, i32, *mut u8, i32) -> i32;
type DefaultLocale = unsafe extern "C" fn() -> *const c_char;

type LocaleForLcid = unsafe extern "C" fn(u32, *mut c_char, i32, *mut i32) -> i32;

type SimpleCase = unsafe extern "C" fn(i32) -> i32;
type GetNormalizer = unsafe extern "C" fn(*mut i32) -> *const c_void;
type Normalize =
    unsafe extern "C" fn(*const c_void, *const u16, i32, *mut u16, i32, *mut i32) -> i32;
type IsNormalized = unsafe extern "C" fn(*const c_void, *const u16, i32, *mut i32) -> i8;

type ToLanguageTag = unsafe extern "C" fn(*const c_char, *mut c_char, i32, i8, *mut i32) -> i32;
type GetLcid = unsafe extern "C" fn(*const c_char) -> u32;
type OpenDateFormat = unsafe extern "C" fn(
    i32,
    i32,
    *const c_char,
    *const u16,
    i32,
    *const u16,
    i32,
    *mut i32,
) -> *mut c_void;
type DatePattern = unsafe extern "C" fn(*const c_void, i8, *mut u16, i32, *mut i32) -> i32;

struct Api {
    open: Open,
    close: Close,
    compare: Compare,
    rules: Rules,
    open_rules: OpenRules,
    open_elements: OpenElements,
    close_elements: Close,
    next: Next,
    previous: Next,
    strength: Strength,
    sort_key: SortKey,
    default_locale: DefaultLocale,
    locale_for_lcid: LocaleForLcid,
    language_tag: ToLanguageTag,
    get_lcid: GetLcid,
    open_date: OpenDateFormat,
    close_date: Close,
    date_pattern: DatePattern,
    upper: SimpleCase,
    lower: SimpleCase,
    normalizers: [GetNormalizer; 4],
    normalize: Normalize,
    is_normalized: IsNormalized,
    _library: Library,
}

impl Api {
    fn load() -> Option<Self> {
        #[cfg(target_os = "macos")]
        let library = Library::open("/usr/lib/libicucore.A.dylib")?;
        #[cfg(not(target_os = "macos"))]
        let library = Library::open("libicui18n.so").or_else(|| {
            (50..=150)
                .rev()
                .find_map(|major| Library::open(&format!("libicui18n.so.{major}")))
        })?;
        let suffix = if library.symbol("ucol_open").is_some() {
            String::new()
        } else {
            (50..=150).rev().find_map(|major| {
                library
                    .symbol(&format!("ucol_open_{major}"))
                    .map(|_| format!("_{major}"))
            })?
        };
        macro_rules! bind {
            ($name:literal, $signature:ty) => {{
                let address = library.symbol(&format!("{}{suffix}", $name))?;
                // SAFETY: these names and signatures are paired below exactly as
                // declared by ICU’s unicode headers. The library outlives all calls.
                unsafe { std::mem::transmute::<*mut c_void, $signature>(address.as_ptr()) }
            }};
        }
        Some(Self {
            open: bind!("ucol_open", Open),
            close: bind!("ucol_close", Close),
            compare: bind!("ucol_strcoll", Compare),
            rules: bind!("ucol_getRules", Rules),
            open_rules: bind!("ucol_openRules", OpenRules),
            open_elements: bind!("ucol_openElements", OpenElements),
            close_elements: bind!("ucol_closeElements", Close),
            next: bind!("ucol_next", Next),
            previous: bind!("ucol_previous", Next),
            strength: bind!("ucol_getStrength", Strength),
            sort_key: bind!("ucol_getSortKey", SortKey),
            default_locale: bind!("uloc_getDefault", DefaultLocale),
            locale_for_lcid: bind!("uloc_getLocaleForLCID", LocaleForLcid),
            language_tag: bind!("uloc_toLanguageTag", ToLanguageTag),
            get_lcid: bind!("uloc_getLCID", GetLcid),
            open_date: bind!("udat_open", OpenDateFormat),
            close_date: bind!("udat_close", Close),
            date_pattern: bind!("udat_toPattern", DatePattern),
            upper: bind!("u_toupper", SimpleCase),
            lower: bind!("u_tolower", SimpleCase),
            normalizers: [
                bind!("unorm2_getNFCInstance", GetNormalizer),
                bind!("unorm2_getNFDInstance", GetNormalizer),
                bind!("unorm2_getNFKCInstance", GetNormalizer),
                bind!("unorm2_getNFKDInstance", GetNormalizer),
            ],
            normalize: bind!("unorm2_normalize", Normalize),
            is_normalized: bind!("unorm2_isNormalized", IsNormalized),
            _library: library,
        })
    }
}

struct Collator {
    handle: NonNull<c_void>,
    api: Rc<Api>,
}

/// An ICU iterator borrows both its collator and the UTF-16 text.
struct Elements<'a> {
    handle: NonNull<c_void>,
    collator: &'a Collator,
    _text: &'a [u16],
}

impl<'a> Elements<'a> {
    fn new(collator: &'a Collator, text: &'a [u16]) -> Option<Self> {
        let length = i32::try_from(text.len()).ok()?;
        let mut status = 0;
        // SAFETY: both inputs stay alive and immutable until this iterator drops.
        let handle = unsafe {
            (collator.api.open_elements)(
                collator.handle.as_ptr(),
                text.as_ptr(),
                length,
                &mut status,
            )
        };
        let elements = Self {
            handle: NonNull::new(handle)?,
            collator,
            _text: text,
        };
        (status <= 0).then_some(elements)
    }

    fn advance(&mut self, forward: bool) -> Option<i32> {
        let step = if forward {
            self.collator.api.next
        } else {
            self.collator.api.previous
        };
        let mut status = 0;
        // SAFETY: step advances this live iterator in its selected direction.
        let value = unsafe { step(self.handle.as_ptr(), &mut status) };
        (status <= 0).then_some(value)
    }
}

impl Drop for Elements<'_> {
    fn drop(&mut self) {
        // SAFETY: the iterator is owned and its collator is still live.
        unsafe { (self.collator.api.close_elements)(self.handle.as_ptr()) };
    }
}

impl Drop for Collator {
    fn drop(&mut self) {
        // SAFETY: the handle came from ucol_open or ucol_openRules and is owned.
        unsafe { (self.api.close)(self.handle.as_ptr()) };
    }
}

impl Collator {
    fn new(api: &Rc<Api>, locale: &str, case_sensitive: bool) -> Option<Self> {
        let name = CString::new(locale).ok()?;
        let mut status = 0;
        // SAFETY: name and status live for the call, and ICU copies the locale.
        let base = Self {
            handle: NonNull::new(unsafe { (api.open)(name.as_ptr(), &mut status) })?,
            api: Rc::clone(api),
        };
        if status > 0 {
            return None;
        }
        if case_sensitive {
            return Some(base);
        }
        // IgnoreCase on .NET still distinguishes kana type and character
        // width. Append the same tailoring to the locale's own collation rules
        // before opening a secondary-strength collator.
        let mut length = 0;
        // SAFETY: ICU owns this view until base is closed. Copy it before then.
        let source = unsafe { (api.rules)(base.handle.as_ptr(), &mut length) };
        let mut rules = if length > 0 {
            let source = NonNull::new(source.cast_mut())?;
            // SAFETY: ucol_getRules returned this many valid UTF-16 units.
            unsafe { std::slice::from_raw_parts(source.as_ptr(), length as usize) }.to_vec()
        } else {
            Vec::new()
        };
        rules.extend(include_str!("ignore_case.rules").encode_utf16());
        let length = i32::try_from(rules.len()).ok()?;
        status = 0;
        // SAFETY: the rules slice contains length units. ICU copies it,
        // UCOL_DEFAULT = -1, UCOL_SECONDARY = 1 and parseError is optional.
        let handle = unsafe {
            (api.open_rules)(rules.as_ptr(), length, -1, 1, ptr::null_mut(), &mut status)
        };
        let tailored = Self {
            handle: NonNull::new(handle)?,
            api: Rc::clone(api),
        };
        (status <= 0).then_some(tailored)
    }

    fn compare(&self, left: &[u16], right: &[u16]) -> Option<Ordering> {
        let left_len = i32::try_from(left.len()).ok()?;
        let right_len = i32::try_from(right.len()).ok()?;
        // SAFETY: both pointers refer to initialized slices of the stated lengths.
        let order = unsafe {
            (self.api.compare)(
                self.handle.as_ptr(),
                left.as_ptr(),
                left_len,
                right.as_ptr(),
                right_len,
            )
        };
        Some(order.cmp(&0))
    }

    fn sort_key(&self, text: &[u16]) -> Option<Vec<u8>> {
        let len = i32::try_from(text.len()).ok()?;
        let mut key = vec![0; text.len().saturating_add(16)];
        loop {
            let capacity = i32::try_from(key.len()).ok()?;
            // SAFETY: the UTF-16 slice and mutable output contain the stated
            // lengths. ICU reports the required size, including the terminator.
            let written = unsafe {
                (self.api.sort_key)(
                    self.handle.as_ptr(),
                    text.as_ptr(),
                    len,
                    key.as_mut_ptr(),
                    capacity,
                )
            };
            let needed = usize::try_from(written).ok().filter(|length| *length > 0)?;
            if needed <= key.len() {
                key.truncate(needed);
                return Some(key);
            }
            key.resize(needed, 0);
        }
    }

    fn has_affix(&self, text: &[u16], pattern: &[u16], prefix: bool) -> Option<bool> {
        let mut pattern = Elements::new(self, pattern)?;
        let mut text = Elements::new(self, text)?;
        // SAFETY: this asks for an attribute of the live collator.
        let strength = unsafe { (self.api.strength)(self.handle.as_ptr()) };
        let mask = match strength {
            0 => !0xffff,
            1 => !0xff,
            _ => -1,
        };
        let mut want = pattern.advance(prefix)?;
        let mut have = text.advance(prefix)?;
        loop {
            if want == -1 {
                // A combining accent immediately following a prefix belongs to
                // its preceding base. An ignorable element can end the match.
                let combining = have & !0xffff == 0 && have & 0xff00 != 0;
                return Some(!prefix || !combining);
            }
            if want == 0 {
                want = pattern.advance(prefix)?;
            } else if have == 0 {
                have = text.advance(prefix)?;
            } else if have == -1 || (want ^ have) & mask != 0 {
                return Some(false);
            } else {
                want = pattern.advance(prefix)?;
                have = text.advance(prefix)?;
            }
        }
    }
}

struct Cache {
    api: Option<Rc<Api>>,
    entries: Vec<(String, bool, Collator)>,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache {
        api: Api::load().map(Rc::new),
        entries: Vec::new(),
    });
}

pub(super) fn default_locale() -> Option<String> {
    CACHE.with_borrow(|cache| {
        let api = cache.api.as_ref()?;
        // SAFETY: ICU owns a NUL-terminated locale ID. We copy it immediately
        // and never call uloc_setDefault.
        let locale = NonNull::new(unsafe { (api.default_locale)() }.cast_mut())?;
        let name = unsafe { CStr::from_ptr(locale.as_ptr()) }.to_str().ok()?;
        // .NET maps ICU's default en_US_POSIX locale to the invariant culture.
        Some(if name == "en_US_POSIX" {
            String::new()
        } else {
            name.to_owned()
        })
    })
}

pub(super) fn locale_for_lcid(id: u32) -> Option<String> {
    CACHE.with_borrow(|cache| {
        let api = cache.api.as_ref()?;
        let mut bytes = [0u8; 157]; // ULOC_FULLNAME_CAPACITY
        let mut status = 0;
        // SAFETY: ICU writes at most the supplied capacity; we use the returned length.
        let len = unsafe {
            (api.locale_for_lcid)(
                id,
                bytes.as_mut_ptr().cast(),
                bytes.len() as i32,
                &mut status,
            )
        };
        if status > 0 || len <= 0 {
            return None;
        }
        std::str::from_utf8(bytes.get(..len as usize)?)
            .ok()
            .map(str::to_owned)
    })
}

fn with_collator<T>(
    locale: &str,
    case_sensitive: bool,
    operation: impl FnOnce(&Collator) -> Option<T>,
) -> Option<T> {
    CACHE.with_borrow_mut(|cache| {
        if let Some((_, _, collator)) = cache
            .entries
            .iter()
            .find(|(name, case, _)| name == locale && *case == case_sensitive)
        {
            return operation(collator);
        }
        let collator = Collator::new(cache.api.as_ref()?, locale, case_sensitive)?;
        let result = operation(&collator);
        if cache.entries.len() == 8 {
            cache.entries.remove(0);
        }
        cache
            .entries
            .push((locale.to_owned(), case_sensitive, collator));
        result
    })
}

pub(super) fn compare(
    left: &[u16],
    right: &[u16],
    locale: &str,
    case_sensitive: bool,
) -> Option<Ordering> {
    with_collator(locale, case_sensitive, |collator| {
        collator.compare(left, right)
    })
}

pub(super) fn sort_key(text: &[u16], locale: &str, case_sensitive: bool) -> Option<Vec<u8>> {
    with_collator(locale, case_sensitive, |collator| collator.sort_key(text))
}

pub(super) fn has_affix(
    text: &[u16],
    pattern: &[u16],
    locale: &str,
    case_sensitive: bool,
    prefix: bool,
) -> Option<bool> {
    with_collator(locale, case_sensitive, |collator| {
        collator.has_affix(text, pattern, prefix)
    })
}

pub(super) fn change_case(text: &PsString, casing: Casing, upper: bool) -> Option<PsString> {
    CACHE.with_borrow(|cache| {
        let api = cache.api.as_ref()?;
        let convert = if upper { api.upper } else { api.lower };
        let units = map_units(text.units(), casing, upper, false, |ch, _| {
            // SAFETY: the simple Unicode case API takes and returns a single UChar32.
            char::from_u32(unsafe { convert(ch as i32) } as u32).unwrap_or(ch)
        });
        Some(PsString::from_units(units))
    })
}

impl Api {
    fn normalizer(&self, form: NormalizationForm) -> Option<NonNull<c_void>> {
        let index = match form {
            NormalizationForm::C => 0,
            NormalizationForm::D => 1,
            NormalizationForm::Kc => 2,
            NormalizationForm::Kd => 3,
        };
        let mut status = 0;
        // SAFETY: ICU owns this immutable singleton for the lifetime of the library.
        let handle = unsafe { (self.normalizers[index])(&mut status) };
        if status > 0 {
            return None;
        }
        NonNull::new(handle.cast_mut())
    }
}

pub(super) fn normalize(text: &[u16], form: NormalizationForm) -> Option<Vec<u16>> {
    CACHE.with_borrow(|cache| {
        let api = cache.api.as_ref()?;
        let normalizer = api.normalizer(form)?;
        let length = i32::try_from(text.len()).ok()?;
        let mut output = vec![0; text.len()];
        loop {
            let mut status = 0;
            let capacity = i32::try_from(output.len()).ok()?;
            // SAFETY: input and output are distinct buffers with the supplied lengths.
            let count = unsafe {
                (api.normalize)(
                    normalizer.as_ptr(),
                    text.as_ptr(),
                    length,
                    output.as_mut_ptr(),
                    capacity,
                    &mut status,
                )
            };
            let needed = usize::try_from(count).ok()?;
            if status == 15 && needed > output.len() {
                // U_BUFFER_OVERFLOW_ERROR
                output.resize(needed, 0);
                continue;
            }
            if status > 0 || needed > output.len() {
                return None;
            }
            output.truncate(needed);
            return Some(output);
        }
    })
}

pub(super) fn is_normalized(text: &[u16], form: NormalizationForm) -> Option<bool> {
    CACHE.with_borrow(|cache| {
        let api = cache.api.as_ref()?;
        let normalizer = api.normalizer(form)?;
        let length = i32::try_from(text.len()).ok()?;
        let mut status = 0;
        // SAFETY: the singleton and immutable UTF-16 slice live until the call returns.
        let result =
            unsafe { (api.is_normalized)(normalizer.as_ptr(), text.as_ptr(), length, &mut status) };
        (status <= 0).then_some(result != 0)
    })
}

pub(super) fn locale_details(name: &str) -> Option<(String, u32)> {
    let locale = CString::new(name).ok()?;
    CACHE.with_borrow(|cache| {
        let api = cache.api.as_ref()?;
        let mut tag = [0u8; 157]; // ULOC_FULLNAME_CAPACITY
        let mut status = 0;
        // SAFETY: locale is NUL-terminated; ICU writes at most the buffer capacity.
        let len = unsafe {
            (api.language_tag)(
                locale.as_ptr(),
                tag.as_mut_ptr().cast(),
                tag.len() as i32,
                0,
                &mut status,
            )
        };
        if status > 0 || len <= 0 {
            return None;
        }
        let tag = std::str::from_utf8(tag.get(..len as usize)?).ok()?;
        let lcid = unsafe { (api.get_lcid)(locale.as_ptr()) };
        Some((tag.to_owned(), if lcid == 0 { 4096 } else { lcid }))
    })
}

pub(super) fn time_pattern(locale: &str, short: bool) -> Option<String> {
    let locale = CString::new(locale).ok()?;
    CACHE.with_borrow(|cache| {
        let api = cache.api.as_ref()?;
        let mut status = 0;
        // SAFETY: UDAT_SHORT / UDAT_MEDIUM with UDAT_NONE for the date;
        // no pattern or timezone buffer is supplied.
        let raw = unsafe {
            (api.open_date)(
                if short { 3 } else { 2 },
                -1,
                locale.as_ptr(),
                ptr::null(),
                0,
                ptr::null(),
                0,
                &mut status,
            )
        };
        let handle = NonNull::new(raw)?;
        let mut pattern = [0u16; 157];
        let len = unsafe {
            (api.date_pattern)(
                handle.as_ptr(),
                0,
                pattern.as_mut_ptr(),
                pattern.len() as i32,
                &mut status,
            )
        };
        // SAFETY: every successful udat_open above owns exactly one handle.
        unsafe { (api.close_date)(handle.as_ptr()) };
        if status > 0 {
            return None;
        }
        String::from_utf16(pattern.get(..usize::try_from(len).ok()?)?).ok()
    })
}
