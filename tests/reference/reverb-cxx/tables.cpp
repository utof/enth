// Dump the RNG draws of MultiDelay::updateLengths for the four constant seeds (libstdc++ distributions).
#include <random>
#include <cstdio>
#include <cstring>
#include <cstdint>
int main() {
    const unsigned seeds[4] = {0x6DD09EE5, 0x876753A5, 0x5974DF44, 0x8CDBF7E6};
    const unsigned early = 0x0BDDE171;
    unsigned all[5] = {seeds[0], seeds[1], seeds[2], seeds[3], early};
    for (unsigned s : all) {
        int seed = int(s); std::mt19937 engine(seed);
        std::uniform_real_distribution<float> unitDist(0, 1);
        printf("seed %08x:", s);
        for (int i = 0; i < 8; ++i) {
            float unit = unitDist(engine);
            std::uniform_int_distribution<int> indexDist(0, i);
            int swapIndex = indexDist(engine);
            uint32_t bits; memcpy(&bits, &unit, 4);
            printf(" (%a=0x%08x,%d)", unit, bits, swapIndex);
        }
        printf("\n");
    }
}
