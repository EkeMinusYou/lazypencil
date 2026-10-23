# Offline icon fixture

`feather-icons-4.29.2.tgz` is the unmodified upstream npm package archive:

- Source: https://registry.npmjs.org/feather-icons/-/feather-icons-4.29.2.tgz
- SHA-256: `28b6cad56600feee25c53051d65326621f2675d30d6f3d9bd3175f6117f84476`
- Copyright: Copyright (c) 2013-2023 Cole Bemis
- License: MIT. The complete notice is inside `package/LICENSE` and retained at [resources/icons/feather/LICENSE](../../../resources/icons/feather/LICENSE).

The archive allows installation, integrity, concurrency, and dependency tests without a network connection. Tests and the installer read selected members without extracting paths or executing package scripts. The archive includes upstream JavaScript files, which lazypencil does not execute or compile into its binary. Retain the upstream license when redistributing this fixture.

`phosphor-core-2.1.1.tgz` is the complete, unchanged upstream `@phosphor-icons/core` npm archive:

- Source: https://registry.npmjs.org/@phosphor-icons/core/-/core-2.1.1.tgz
- SHA-256: `313332be6190b724da24107addd781799b48bf76b13963f24501112ffe1baadd`
- Copyright: Copyright (c) 2023 Phosphor Icons.
- License: MIT. Preserve the complete original `package/LICENSE`, also retained at [resources/icons/phosphor/LICENSE](../../../resources/icons/phosphor/LICENSE).

The regular paths and source catalogue are tested without executing bundled package code. The fixture contains all upstream weights and JavaScript because the original archive is retained unchanged; the current `.pen` `phosphor` pack resolves the regular paths. Its index records PascalCase and deprecated names as aliases, rather than copying unrelated code into lazypencil.

Material's full original fonts are deliberately absent from this fixture directory. Opt-in tests accept separately obtained pinned TTFs and original `.codepoints` files through `LAZYPENCIL_TEST_MATERIAL_DIR`; the hashes and Apache-2.0 attribution are pinned in [the catalogue](../../../resources/icons/catalogue.json) and the resource provenance files.
