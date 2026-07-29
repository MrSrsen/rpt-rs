# rpt-rs documentation

Five sets of pages, one per domain. Each set has its own index and its own reading order, and every page links to the
next one in **its own** domain — so pick the domain you need and read it through. Taken in the order below, the five
sets are also the whole documentation front to back.

| Domain                                          | What it covers                                                                                                                | Read it if                                                                 |
|-------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------|----------------------------------------------------------------------------|
| [**format/**](format/README.md)                 | The `.rpt` binary format: container, cipher, record tree, record catalog, endianness. Programming-language-agnostic.          | You want to know what is in the file — whatever language you read it with. |
| [**reader/**](reader/README.md)                 | The `rpt-reader` crate and the `rpt` binary: the typed report model, what the decoder supports, the CLI, and the library API. | You are inspecting or exporting reports with rpt-rs.                       |
| [**rendering/**](rendering/README.md)           | The render pipeline: data → layout → Page IR → PDF, the render API, the `rpt-render` CLI, and how the renderer is tested.     | You are rendering reports or working on the pipeline.                      |
| [**formula-engine/**](formula-engine/README.md) | The `rpt-formula` crate: the Crystal/Basic formula language, its VM, its builtins, and its validator.                         | You are evaluating, validating, or extending formulas.                     |
| [**project/**](project/README.md)               | rpt-rs itself: the crate/module map and its boundaries, and how to build, test and release it.                                | You are working on rpt-rs rather than with it.                             |

---

**Start here:** [The `.rpt` format](format/README.md) →
