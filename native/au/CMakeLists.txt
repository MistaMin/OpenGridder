cmake_minimum_required(VERSION 3.22)
project(OpenGridderAU LANGUAGES CXX)
set(CMAKE_CXX_STANDARD 11)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

set(OPENGRIDDER_RUST_LIBRARY "" CACHE FILEPATH "Architecture-matched Rust client library")
if(NOT EXISTS "${OPENGRIDDER_RUST_LIBRARY}")
  message(FATAL_ERROR "Build Rust library and set OPENGRIDDER_RUST_LIBRARY")
endif()

add_library(OpenGridder MODULE OpenGridderAU.cpp)
target_include_directories(OpenGridder PRIVATE ../include)
target_link_libraries(OpenGridder PRIVATE
  "${OPENGRIDDER_RUST_LIBRARY}"
  "-framework AudioUnit"
  "-framework AudioToolbox"
  "-framework CoreAudio"
  "-framework CoreFoundation"
  "-framework Security"
  "-framework SystemConfiguration"
  iconv resolv
)

if(CMAKE_OSX_ARCHITECTURES STREQUAL "i386")
  target_link_libraries(OpenGridder PRIVATE "-framework CoreServices")
endif()

set(BUNDLE_DIR "${CMAKE_BINARY_DIR}/OpenGridder.component")
set_target_properties(OpenGridder PROPERTIES
  PREFIX ""
  SUFFIX ""
  LIBRARY_OUTPUT_DIRECTORY "${BUNDLE_DIR}/Contents/MacOS"
)

file(MAKE_DIRECTORY "${BUNDLE_DIR}/Contents/Resources")
configure_file(Info.plist.in "${BUNDLE_DIR}/Contents/Info.plist" @ONLY)
file(WRITE "${BUNDLE_DIR}/Contents/PkgInfo" "BNDL????")
