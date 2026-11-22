cmake_minimum_required(VERSION 3.20)
project(xholy_window_manager LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

if(NOT WIN32)
    message(FATAL_ERROR "window_manager requires Windows (Win32 API)")
endif()

# High-DPI awareness for the test binary.
add_compile_definitions(
    UNICODE
    _UNICODE
    WIN32_LEAN_AND_MEAN
    NOMINMAX
)

add_library(wm_core STATIC
    window_manager.cpp
)
set_target_properties(wm_core PROPERTIES OUTPUT_NAME window_manager)
target_include_directories(wm_core PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})
target_link_libraries(wm_core PUBLIC user32 gdi32 shell32)

add_executable(window_manager_tests
    window_manager_tests.cpp
)
target_link_libraries(window_manager_tests PRIVATE wm_core)

add_executable(window_manager WIN32
    main.cpp
)
target_link_libraries(window_manager PRIVATE wm_core)

enable_testing()
add_test(NAME window_manager_tests COMMAND window_manager_tests)
