cmake_minimum_required(VERSION 3.15)
project(OpenGridderVST3 LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

set(VST3_SDK_ROOT "/Users/marcosdeida/Projects/audiogridder/JUCE/modules/juce_audio_processors_headless/format_types/VST3_SDK" CACHE PATH "Steinberg VST3 SDK Root")

add_library(OpenGridder MODULE
    OpenGridderVST3.cpp
    "${VST3_SDK_ROOT}/pluginterfaces/base/funknown.cpp"
    "${VST3_SDK_ROOT}/pluginterfaces/base/coreiids.cpp"
    "${VST3_SDK_ROOT}/public.sdk/source/vst/vstinitiids.cpp"
)

set_target_properties(OpenGridder PROPERTIES
    BUNDLE TRUE
    BUNDLE_EXTENSION "vst3"
    OUTPUT_NAME "OpenGridder"
    MACOSX_BUNDLE_INFO_PLIST "${CMAKE_CURRENT_SOURCE_DIR}/Info.plist.in"
    XCODE_ATTRIBUTE_WRAPPER_EXTENSION "vst3"
    PREFIX ""
    SUFFIX ""
)

target_include_directories(OpenGridder PRIVATE
    "${VST3_SDK_ROOT}"
    "${CMAKE_CURRENT_SOURCE_DIR}/../include"
)

if(DEFINED OPENGRIDDER_RUST_LIBRARY)
    target_link_libraries(OpenGridder PRIVATE "${OPENGRIDDER_RUST_LIBRARY}")
endif()

if(APPLE)
    target_link_libraries(OpenGridder PRIVATE
        "-framework CoreFoundation"
        "-framework Security"
    )
    set_target_properties(OpenGridder PROPERTIES
        LINK_FLAGS "-exported_symbols_list ${CMAKE_CURRENT_SOURCE_DIR}/vst3_exports.exp"
    )
endif()
