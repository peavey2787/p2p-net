cmake_minimum_required(VERSION 3.22.1)
project(p2p_android_jni LANGUAGES CXX)

set(RUST_LIB_DIR "${CMAKE_CURRENT_LIST_DIR}/../jniLibs/${ANDROID_ABI}")

add_library(p2p_net_android SHARED IMPORTED)
set_target_properties(
    p2p_net_android
    PROPERTIES
        IMPORTED_LOCATION "${RUST_LIB_DIR}/libp2p_net_android.so"
        # Rust cdylibs do not carry an ELF SONAME by default. Tell CMake that
        # explicitly so it links with -lp2p_net_android instead of embedding
        # this host build path into p2p_android_jni's DT_NEEDED table.
        IMPORTED_NO_SONAME TRUE
)

add_library(p2p_android_jni SHARED p2p_jni.cpp)
target_compile_features(p2p_android_jni PRIVATE cxx_std_20)
target_compile_options(p2p_android_jni PRIVATE -Wall -Wextra -Werror -fvisibility=hidden)
set_target_properties(
    p2p_android_jni
    PROPERTIES
        # Android resolves packaged JNI dependencies from the app namespace.
        # A host build directory must never become an ELF RUNPATH.
        SKIP_BUILD_RPATH TRUE
)
target_link_libraries(p2p_android_jni PRIVATE p2p_net_android log android)
