// FX1 golden harness: renders Signalsmith Basics' reverb (upstream headers, unmodified) for the scenarios of
// crates/enth-dsp/tests/fixtures/reverb-scenarios.txt and writes interleaved little-endian float32 (L R L R ...).
//
// Usage: harness <scenarios.txt> <scenario-name> <out.f32>
//        harness <scenarios.txt> --list
//
// Spec format (one directive per line, '#' starts a comment):
//   scenario <name> <sample_rate> <block_frames> <seconds>
//   set <key>=<value> ...          initial parameters (everything else keeps upstream's default)
//   change <ms> <key>=<value> ...  applied before the first block that starts at or after <ms> (integer maths:
//                                  frame = sample_rate * ms / 1000)
// Keys (the enth units; dry, wet and early are percent of gain, divided by 100.0 here exactly as the Rust test
// does): dry_pct wet_pct room_ms rt20_s early_pct detune low_cut_hz high_cut_hz low_damp_rate high_damp_rate.
//
// Input: SplitMix64 noise burst in the first 250 ms, then an impulse (1.0, -0.5) at 1.5 s, silence otherwise.
#include "signalsmith-basics/reverb.h"
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>

static uint64_t sm_state;
static uint64_t sm_next() {
	sm_state += 0x9E3779B97F4A7C15ull;
	uint64_t z = sm_state;
	z = (z ^ (z >> 30)) * 0xBF58476D1CE4E5B9ull;
	z = (z ^ (z >> 27)) * 0x94D049BB133111EBull;
	return z ^ (z >> 31);
}
static float sm_unit() { return float(sm_next() >> 40) / 16777216.0f; }

using Verb = signalsmith::basics::ReverbFloat;

struct Change {
	long ms;
	std::vector<std::pair<std::string, double>> kv;
};
struct Scenario {
	std::string name;
	int sampleRate = 48000, block = 64, seconds = 4;
	std::vector<std::pair<std::string, double>> init;
	std::vector<Change> changes;
};

static void apply(Verb &v, const std::string &key, double x) {
	if (key == "dry_pct") v.dry = x / 100.0;
	else if (key == "wet_pct") v.wet = x / 100.0;
	else if (key == "room_ms") v.roomMs = x;
	else if (key == "rt20_s") v.rt20 = x;
	else if (key == "early_pct") v.early = x / 100.0;
	else if (key == "detune") v.detune = x;
	else if (key == "low_cut_hz") v.lowCutHz = x;
	else if (key == "high_cut_hz") v.highCutHz = x;
	else if (key == "low_damp_rate") v.lowDampRate = x;
	else if (key == "high_damp_rate") v.highDampRate = x;
	else { fprintf(stderr, "unknown key %s\n", key.c_str()); exit(2); }
}

static std::vector<std::pair<std::string, double>> parseKv(std::istringstream &ss) {
	std::vector<std::pair<std::string, double>> out;
	std::string tok;
	while (ss >> tok) {
		size_t eq = tok.find('=');
		if (eq == std::string::npos) { fprintf(stderr, "bad token %s\n", tok.c_str()); exit(2); }
		out.emplace_back(tok.substr(0, eq), strtod(tok.c_str() + eq + 1, nullptr));
	}
	return out;
}

static std::vector<Scenario> load(const char *path) {
	std::ifstream f(path);
	if (!f) { fprintf(stderr, "cannot read %s\n", path); exit(2); }
	std::vector<Scenario> all;
	std::string line;
	while (std::getline(f, line)) {
		size_t hash = line.find('#');
		if (hash != std::string::npos) line.resize(hash);
		std::istringstream ss(line);
		std::string word;
		if (!(ss >> word)) continue;
		if (word == "scenario") {
			Scenario s;
			ss >> s.name >> s.sampleRate >> s.block >> s.seconds;
			all.push_back(s);
		} else if (word == "set") {
			all.back().init = parseKv(ss);
		} else if (word == "change") {
			Change c;
			ss >> c.ms;
			c.kv = parseKv(ss);
			all.back().changes.push_back(c);
		} else { fprintf(stderr, "unknown directive %s\n", word.c_str()); exit(2); }
	}
	return all;
}

int main(int argc, char **argv) {
	if (argc < 3) { fprintf(stderr, "usage: harness <scenarios.txt> <name|--list> [out.f32]\n"); return 2; }
	std::vector<Scenario> all = load(argv[1]);
	if (!strcmp(argv[2], "--list")) {
		for (auto &s : all) printf("%s\n", s.name.c_str());
		return 0;
	}
	const Scenario *sc = nullptr;
	for (auto &s : all) if (s.name == argv[2]) sc = &s;
	if (!sc || argc < 4) { fprintf(stderr, "unknown scenario or missing output path\n"); return 2; }

	const int sr = sc->sampleRate, block = sc->block, frames = sr * sc->seconds;
	Verb verb; // maxRoomMs 200, detuneDepthMs 2: upstream's defaults
	for (auto &kv : sc->init) apply(verb, kv.first, kv.second);
	verb.configure(sr, block, 2);

	sm_state = 12345;
	std::vector<float> inL(frames), inR(frames), out(2 * size_t(frames));
	for (int i = 0; i < frames; ++i) {
		float l = 0, r = 0;
		if (i < sr / 4) { l = (sm_unit() - 0.5f) * 0.5f; r = (sm_unit() - 0.5f) * 0.5f; }
		if (i == sr * 3 / 2) { l = 1.0f; r = -0.5f; }
		inL[i] = l; inR[i] = r;
	}
	std::vector<float> oL(block), oR(block);
	std::vector<bool> done(sc->changes.size(), false);
	for (int pos = 0; pos < frames; pos += block) {
		for (size_t c = 0; c < sc->changes.size(); ++c) {
			long at = long(sr) * sc->changes[c].ms / 1000;
			if (!done[c] && pos >= at) {
				for (auto &kv : sc->changes[c].kv) apply(verb, kv.first, kv.second);
				done[c] = true;
			}
		}
		int n = block < frames - pos ? block : frames - pos;
		float *ins[2] = {inL.data() + pos, inR.data() + pos};
		float *outs[2] = {oL.data(), oR.data()};
		verb.process(ins, outs, n);
		for (int i = 0; i < n; ++i) { out[2 * size_t(pos + i)] = oL[i]; out[2 * size_t(pos + i) + 1] = oR[i]; }
	}
	FILE *f = fopen(argv[3], "wb");
	fwrite(out.data(), sizeof(float), out.size(), f);
	fclose(f);
	return 0;
}
