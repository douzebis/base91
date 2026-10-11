{
  lib,
  base91,
  buildPythonPackage,
  rustPlatform,
  pytestCheckHook,
}:

buildPythonPackage (finalAttrs: {
  pname = "pybase91";
  # The same source and vendored crates as the base91 command-line tool:
  # updating base91 updates both.
  inherit (base91) version src cargoDeps;
  pyproject = true;

  __structuredAttrs = true;

  cargoRoot = "rust";
  buildAndTestSubdir = "rust/base91";

  nativeBuildInputs = [
    rustPlatform.cargoSetupHook
    rustPlatform.maturinBuildHook
  ];

  nativeCheckInputs = [ pytestCheckHook ];
  enabledTestPaths = [ "rust/base91/python-tests" ];

  pythonImportsCheck = [ "pybase91" ];

  meta = {
    description = "Python bindings to base91-rs, a basE91 encoder and decoder";
    homepage = "https://github.com/douzebis/base91";
    changelog = "https://github.com/douzebis/base91/blob/v${finalAttrs.version}/CHANGELOG.md";
    license = lib.licenses.mit;
    maintainers = with lib.maintainers; [ douzebis ];
  };
})
