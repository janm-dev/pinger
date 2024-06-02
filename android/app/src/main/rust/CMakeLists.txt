cmake_minimum_required(VERSION 3.22)
project(pinger-lib)

include(FetchContent)

FetchContent_Declare(
	Corrosion
	GIT_REPOSITORY https://github.com/corrosion-rs/corrosion.git
	GIT_TAG v0.5
)
FetchContent_MakeAvailable(Corrosion)

corrosion_import_crate(MANIFEST_PATH ../../../../../lib/Cargo.toml)

add_library(pinger-lib SHARED dummy.cpp)
target_link_libraries(pinger-lib PUBLIC pinger)

corrosion_set_features(
	pinger
	FEATURES java-ffi
)
