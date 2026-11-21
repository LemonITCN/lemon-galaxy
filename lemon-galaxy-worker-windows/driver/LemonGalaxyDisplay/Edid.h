// Builds the 128-byte EDID of a virtual monitor. Windows identifies a monitor by its EDID
// (manufacturer, product code, serial), names it after the descriptor string and derives its
// default scaling from the physical size, so each slot gets its own.
#pragma once

#include <array>
#include <cstdint>
#include <string>

namespace lg {

struct EdidInfo {
    std::wstring name;     // up to 13 ASCII characters are kept
    uint16_t productCode = 1;
    uint32_t serial = 1;
    uint32_t widthMm = 0;
    uint32_t heightMm = 0;
    uint32_t width = 1920;  // preferred mode
    uint32_t height = 1080;
    uint32_t refreshRate = 60;
};

using Edid = std::array<uint8_t, 128>;

Edid BuildEdid(const EdidInfo& info);

}  // namespace lg
