if(NOT PROJECT_NAME STREQUAL "whisper.cpp")
    return()
endif()
include_guard(GLOBAL)
find_program(VTD_GIT git REQUIRED)
set(patch "${CMAKE_CURRENT_LIST_DIR}/../patches/windows/whisper.patch")
execute_process(COMMAND "${VTD_GIT}" apply --reverse --check "${patch}"
    WORKING_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}" RESULT_VARIABLE applied OUTPUT_QUIET ERROR_QUIET)
if(NOT applied EQUAL 0)
    execute_process(COMMAND "${VTD_GIT}" apply --check "${patch}"
        WORKING_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}" COMMAND_ERROR_IS_FATAL ANY)
    execute_process(COMMAND "${VTD_GIT}" apply "${patch}"
        WORKING_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}" COMMAND_ERROR_IS_FATAL ANY)
endif()
add_compile_options(/Gy /Gw)
