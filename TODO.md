# TODO

## Grammar Issues

### Groovy Grammar
- Tests in `src/languages/groovy/utils.rs` are skipped when the Groovy parser is not available
- The test `test_definition_bug_method_invocation_parts` specifically tests for method invocation part identification

### Method Resolution Fixes Applied

#### Common (Java/Groovy)
- Fixed static method resolution in `src/languages/common/method_resolution.rs`: When cursor is on the class name (e.g., `JsonUtil` in `JsonUtil.toString()`), it now correctly jumps to the class definition instead of the method
- Fixed instance method resolution in `src/languages/common/method_resolution.rs`: When cursor is on the variable/instance name (e.g., `myInstance` in `myInstance.doSomething()`), it now correctly jumps to the variable declaration instead of the method

#### Kotlin Specific  
- Fixed static method resolution in `src/languages/kotlin/support.rs`: Updated `extract_static_method_context` to check if cursor is on class name vs method name
- Fixed instance method resolution in `src/languages/kotlin/support.rs`: Updated `extract_instance_method_context` to check if cursor is on variable name vs method name
- Implemented constructor parameter resolution: 
  - Added `find_parameter_type` implementation to handle `class_parameter` nodes (not just `formal_parameter`)
  - Enhanced local resolution in `src/languages/kotlin/definition/local.rs` to find constructor parameters via `find_containing_class` and `find_constructor_parameters`
- Simplified instance method resolution to jump directly to variable declaration instead of complex method resolution
- Added comprehensive tests for static method, instance method, and constructor parameter resolution scenarios

### All Fixes Complete
- ✅ Java method resolution (via common module)
- ✅ Groovy method resolution (via common module)  
- ✅ Kotlin method resolution (via language-specific implementations)

All three JVM languages now correctly handle:
- Cursor on class/variable name → jumps to class/variable definition
- Cursor on method name → jumps to method definition

### Code Cleanup Completed
- Removed all debug logs added during development
- Removed unused `find_instance_method_definition` function and related helper functions
- Updated Java and Groovy support to use simplified variable resolution
- Fixed compilation errors and warnings
- All tests continue to pass