# 第三方组件说明

ClipRaft 原创代码采用 [MIT 许可证](LICENSE)。第三方组件保留各自的版权与许可证，不能因项目采用 MIT 而忽略其要求。

## 依赖来源

前端依赖由 `package-lock.json` 锁定，Rust 及原生组件依赖由 `src-tauri/Cargo.lock` 锁定。从源码构建时，应保留各依赖发行包中的版权与许可文本。

本地二进制 ZIP 随附 `THIRD-PARTY/`，包含依赖清单、许可文本、原生组件许可及相关源码归档。清单是保守的依赖集合，包含部分构建期组件，不表示所有条目均进入最终二进制。SQLCipher/SQLite、OpenSSL、zstd 等原生组件的许可材料一并保留。

## MPL-2.0 组件

当前锁定依赖包含以下采用 MPL-2.0 的组件。它们按原许可使用，项目整体的 MIT 授权不会覆盖这些组件。

| 组件 | 版本 | 对应源码 |
| --- | --- | --- |
| cssparser | `0.37.0` | [crates.io](https://crates.io/crates/cssparser/0.37.0) |
| cssparser-macros | `0.7.1` | [crates.io](https://crates.io/crates/cssparser-macros/0.7.1) |
| dtoa-short | `0.3.5` | [crates.io](https://crates.io/crates/dtoa-short/0.3.5) |
| option-ext | `0.2.0` | [crates.io](https://crates.io/crates/option-ext/0.2.0) |
| selectors | `0.38.0` | [crates.io](https://crates.io/crates/selectors/0.38.0) |

本地 ZIP 的 `THIRD-PARTY/sources/` 随附上述精确版本的未修改源码归档，许可文本位于 `THIRD-PARTY/rust/`。如以后修改这些组件的受许可文件，分发时需继续满足 MPL-2.0 对相应源码的要求。
