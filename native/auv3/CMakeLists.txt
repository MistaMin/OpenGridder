cmake_minimum_required(VERSION 3.22)
project(OpenGridderAUv3 LANGUAGES OBJCXX CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

set(OPENGRIDDER_RUST_LIBRARY "" CACHE FILEPATH "Architecture-matched Rust client library")
if(NOT EXISTS "${OPENGRIDDER_RUST_LIBRARY}")
    message(FATAL_ERROR "Build Rust library and set OPENGRIDDER_RUST_LIBRARY")
endif()

add_library(OpenGridderAUv3 MODULE
    OpenGridderAUv3.mm
    "${CMAKE_CURRENT_SOURCE_DIR}/../include/OpenGridderEditorView.mm"
)
target_compile_options(OpenGridderAUv3 PRIVATE -fobjc-arc)

set(BUNDLE_DIR "${CMAKE_BINARY_DIR}/OpenGridderAUv3.appex")
set_target_properties(OpenGridderAUv3 PROPERTIES
    PREFIX ""
    SUFFIX ""
    LIBRARY_OUTPUT_DIRECTORY "${BUNDLE_DIR}/Contents/MacOS"
    OUTPUT_NAME "OpenGridderAUv3"
)

target_include_directories(OpenGridderAUv3 PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/../include"
)

target_link_libraries(OpenGridderAUv3 PRIVATE
    "${OPENGRIDDER_RUST_LIBRARY}"
    "-framework Cocoa"
    "-framework AppKit"
    "-framework AudioToolbox"
    "-framework AVFoundation"
    "-framework CoreAudio"
    "-framework Foundation"
    "-framework CoreFoundation"
    "-framework Security"
    "-framework SystemConfiguration"
    iconv resolv
)

file(MAKE_DIRECTORY "${BUNDLE_DIR}/Contents/Resources")
configure_file(Info.plist.in "${BUNDLE_DIR}/Contents/Info.plist" @ONLY)
file(WRITE "${BUNDLE_DIR}/Contents/PkgInfo" "XPC!????")
