//! A temporary resolver owned by the helper's dynamic-store session. Closing
//! the session (including helper death) removes only this resolver, never saved DNS.
use super::HelperFailure;
use core_foundation::{
    array::CFArray,
    base::{CFRelease, CFType, TCFType},
    dictionary::CFDictionary,
    number::CFNumber,
    string::CFString,
};
use system_configuration_sys::dynamic_store::{
    SCDynamicStoreAddTemporaryValue, SCDynamicStoreCreate, SCDynamicStoreRef,
};

pub(super) struct DnsLease(SCDynamicStoreRef);
impl DnsLease {
    pub fn activate(device: &str) -> Result<Self, HelperFailure> {
        let name = CFString::new("Verge TUN DNS lease");
        // SAFETY: no callback/context, borrowed CF arguments remain alive for the call.
        let store = unsafe {
            SCDynamicStoreCreate(
                std::ptr::null(),
                name.as_concrete_TypeRef(),
                None,
                std::ptr::null_mut(),
            )
        };
        if store.is_null() {
            return Err(HelperFailure::new(
                "tun_dns_failed",
                "Cannot open DNS session",
            ));
        }
        let lease = Self(store);
        let key = CFString::new(&format!(
            "State:/Network/Service/com.zzzgydi.verge.{device}/DNS"
        ));
        let value = resolver();
        // This API refuses existing keys; it never overwrites another session's DNS.
        let added = unsafe {
            SCDynamicStoreAddTemporaryValue(store, key.as_concrete_TypeRef(), value.as_CFTypeRef())
        };
        if added == 0 {
            return Err(HelperFailure::new(
                "tun_dns_failed",
                "Cannot add temporary TUN resolver",
            ));
        }
        Ok(lease)
    }
}
impl Drop for DnsLease {
    fn drop(&mut self) {
        // SAFETY: we exclusively own the +1 reference returned by Create.
        unsafe { CFRelease(self.0.cast()) };
    }
}
fn resolver() -> CFDictionary<CFString, CFType> {
    CFDictionary::from_CFType_pairs(&[
        (
            CFString::new("ServerAddresses"),
            CFArray::from_CFTypes(&[CFString::new("198.18.0.2")]).as_CFType(),
        ),
        (
            CFString::new("SupplementalMatchDomains"),
            CFArray::from_CFTypes(&[CFString::new("")]).as_CFType(),
        ),
        (
            CFString::new("SupplementalMatchOrders"),
            CFArray::from_CFTypes(&[CFNumber::from(0i32)]).as_CFType(),
        ),
    ])
}
