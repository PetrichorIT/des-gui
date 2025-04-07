use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

use des::prelude::ModuleRef;

pub struct PropReader {
    inner: Box<dyn PropRead>,
}

impl PropReader {
    pub fn from_key(key: &str, module: &ModuleRef) -> Option<Self> {
        macro_rules! try_ty {
            ($($t:ty),*) => {
                $(
                    if let Ok(Some(prop)) = module.prop::<$t>(key).map(|v| v.present()) {
                        return Some(Self {
                            inner: Box::new(prop),
                        });
                    }
                )*
            };
        }

        try_ty!(
            i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f64, f32, bool, char,
            String
        );

        None
    }

    pub fn get_f64(&mut self) -> Option<f64> {
        self.inner.read_f64()
    }

    pub fn get_str(&mut self) -> String {
        self.inner.read_str()
    }
}

pub trait PropRead {
    fn read_f64(&mut self) -> Option<f64>;
    fn read_str(&mut self) -> String;
}

macro_rules! as_f64_impl {
    ($($t:ty),*) => {
        $(
            impl PropRead for ::des::net::module::Prop<$t, ::des::net::module::Present> {
                fn read_f64(&mut self) -> Option<f64> {
                    Some(self.get() as f64)
                }
                fn read_str(&mut self) -> String {
                    self.get().to_string()
                }
            }
        )*
    };
}

as_f64_impl!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f64, f32
);

macro_rules! as_none_impl {
    ($($t:ty),*) => {
        $(
            impl PropRead for ::des::net::module::Prop<$t, ::des::net::module::Present> {
                fn read_f64(&mut self) -> Option<f64> {
                    None
                }
                fn read_str(&mut self) -> String {
                    self.get().to_string()
                }
            }
        )*
    };
}

as_none_impl!(
    bool,
    char,
    String,
    Ipv4Addr,
    Ipv6Addr,
    IpAddr,
    SocketAddrV4,
    SocketAddrV6,
    SocketAddr
);
