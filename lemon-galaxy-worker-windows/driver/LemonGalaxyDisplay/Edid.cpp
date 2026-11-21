#include "Edid.h"

#include <algorithm>
#include <cmath>

namespace lg {
namespace {

// Three-letter PNP manufacturer id "LGX", packed into five-bit letters (big endian).
constexpr uint16_t kManufacturerId = ((('L' - 'A' + 1) << 10) | (('G' - 'A' + 1) << 5) | ('X' - 'A' + 1));

void WriteString(uint8_t* descriptor, uint8_t tag, const std::wstring& text) {
    // Display descriptor: 0 0 0 tag 0, then 13 bytes of text ended by LF and padded with spaces.
    descriptor[0] = descriptor[1] = descriptor[2] = 0;
    descriptor[3] = tag;
    descriptor[4] = 0;
    size_t length = 0;
    for (wchar_t c : text) {
        if (length == 13) break;
        descriptor[5 + length++] = (c >= 0x20 && c < 0x7F) ? static_cast<uint8_t>(c) : '?';
    }
    if (length < 13) descriptor[5 + length++] = 0x0A;
    while (length < 13) descriptor[5 + length++] = 0x20;
}

// Detailed timing with CVT reduced blanking, which keeps the pixel clock low enough for large
// panels. The OS takes the actual modes from the driver; this one only has to be plausible.
void WriteDetailedTiming(uint8_t* d, const EdidInfo& info) {
    uint32_t width = std::min<uint32_t>(info.width, 4095);
    uint32_t height = std::min<uint32_t>(info.height, 4095);
    uint32_t refresh = info.refreshRate ? info.refreshRate : 60;

    const uint32_t hFront = 48, hSync = 32, hBack = 80;
    const uint32_t hBlank = hFront + hSync + hBack;
    // At least 460 us of vertical blanking.
    const double k = 460e-6 * refresh;
    uint32_t vBlank = static_cast<uint32_t>(std::ceil(k * height / (1.0 - k)));
    vBlank = std::max<uint32_t>(vBlank, 3 + 6 + 6);
    const uint32_t vFront = 3, vSync = 6;
    const uint32_t vBack = vBlank - vFront - vSync;

    uint64_t clock10kHz = static_cast<uint64_t>(width + hBlank) * (height + vBlank) * refresh / 10000;
    while (clock10kHz > 0xFFFF && refresh > 24) {
        refresh /= 2;
        clock10kHz = static_cast<uint64_t>(width + hBlank) * (height + vBlank) * refresh / 10000;
    }
    clock10kHz = std::max<uint64_t>(clock10kHz, 1);

    d[0] = static_cast<uint8_t>(clock10kHz & 0xFF);
    d[1] = static_cast<uint8_t>((clock10kHz >> 8) & 0xFF);
    d[2] = static_cast<uint8_t>(width & 0xFF);
    d[3] = static_cast<uint8_t>(hBlank & 0xFF);
    d[4] = static_cast<uint8_t>(((width >> 8) & 0x0F) << 4 | ((hBlank >> 8) & 0x0F));
    d[5] = static_cast<uint8_t>(height & 0xFF);
    d[6] = static_cast<uint8_t>(vBlank & 0xFF);
    d[7] = static_cast<uint8_t>(((height >> 8) & 0x0F) << 4 | ((vBlank >> 8) & 0x0F));
    d[8] = static_cast<uint8_t>(hFront & 0xFF);
    d[9] = static_cast<uint8_t>(hSync & 0xFF);
    d[10] = static_cast<uint8_t>((vFront & 0x0F) << 4 | (vSync & 0x0F));
    d[11] = static_cast<uint8_t>(((hFront >> 8) & 0x03) << 6 | ((hSync >> 8) & 0x03) << 4 |
                                 ((vFront >> 4) & 0x03) << 2 | ((vSync >> 4) & 0x03));
    const uint32_t widthMm = std::min<uint32_t>(info.widthMm, 4095);
    const uint32_t heightMm = std::min<uint32_t>(info.heightMm, 4095);
    d[12] = static_cast<uint8_t>(widthMm & 0xFF);
    d[13] = static_cast<uint8_t>(heightMm & 0xFF);
    d[14] = static_cast<uint8_t>(((widthMm >> 8) & 0x0F) << 4 | ((heightMm >> 8) & 0x0F));
    d[15] = 0;  // no border
    d[16] = 0;
    d[17] = 0x1E;  // digital separate sync, positive polarities
    (void)vBack;
}

}  // namespace

Edid BuildEdid(const EdidInfo& info) {
    Edid e{};
    // Header.
    const uint8_t header[8] = {0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00};
    std::copy(header, header + 8, e.begin());
    // Vendor and product.
    e[8] = static_cast<uint8_t>(kManufacturerId >> 8);
    e[9] = static_cast<uint8_t>(kManufacturerId & 0xFF);
    e[10] = static_cast<uint8_t>(info.productCode & 0xFF);
    e[11] = static_cast<uint8_t>(info.productCode >> 8);
    e[12] = static_cast<uint8_t>(info.serial & 0xFF);
    e[13] = static_cast<uint8_t>((info.serial >> 8) & 0xFF);
    e[14] = static_cast<uint8_t>((info.serial >> 16) & 0xFF);
    e[15] = static_cast<uint8_t>((info.serial >> 24) & 0xFF);
    e[16] = 0;                // week of manufacture: unknown
    e[17] = 2026 - 1990;      // year of manufacture
    e[18] = 1;                // EDID 1.4
    e[19] = 4;
    e[20] = 0xA5;             // digital input, 8 bits per colour, DisplayPort
    e[21] = static_cast<uint8_t>(std::min<uint32_t>((info.widthMm + 5) / 10, 255));   // cm
    e[22] = static_cast<uint8_t>(std::min<uint32_t>((info.heightMm + 5) / 10, 255));
    e[23] = 120;              // gamma 2.2
    e[24] = 0x06;             // sRGB default colour space, preferred timing is native
    // sRGB chromaticity.
    const uint8_t chroma[10] = {0xEE, 0x91, 0xA3, 0x54, 0x4C, 0x99, 0x26, 0x0F, 0x50, 0x54};
    std::copy(chroma, chroma + 10, e.begin() + 25);
    // No established timings, no standard timings.
    e[35] = e[36] = e[37] = 0;
    for (int i = 38; i < 54; i += 2) {
        e[i] = 0x01;
        e[i + 1] = 0x01;
    }
    WriteDetailedTiming(&e[54], info);
    WriteString(&e[72], 0xFC, info.name.empty() ? std::wstring(L"LemonGalaxy") : info.name);
    wchar_t serialText[16];
    swprintf_s(serialText, L"%08X", info.serial);
    WriteString(&e[90], 0xFF, serialText);
    // Dummy descriptor.
    e[108] = e[109] = e[110] = 0;
    e[111] = 0x10;
    e[126] = 0;  // no extensions
    uint8_t sum = 0;
    for (int i = 0; i < 127; ++i) sum = static_cast<uint8_t>(sum + e[i]);
    e[127] = static_cast<uint8_t>(0x100 - sum);
    return e;
}

}  // namespace lg
