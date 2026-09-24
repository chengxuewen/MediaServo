// app_model — 数据工具实现（T3 自 main.cpp 匿名 namespace 迁居，逐字保真）。
#include "app_model.hpp"

#include <cstdlib>

namespace viewer {

const char* env_or(const char* k, const char* dflt) {
    const char* v = std::getenv(k);
    return (v && *v) ? v : dflt;
}

/// list_rooms JSON 数组的逐项提取（键均字符串形，手工扫描零依赖）。
std::vector<std::pair<std::string, std::string>> parse_rooms(const std::string& json) {
    std::vector<std::pair<std::string, std::string>> out;
    std::string::size_type pos = 0;
    while ((pos = json.find("\"room_id\"", pos)) != std::string::npos) {
        auto q1 = json.find('"', pos + 9); // pat 尾后起（+8 会自撞 pat 闭引号=空串 bug）
        auto q2 = json.find('"', q1 + 1);
        if (q1 == std::string::npos || q2 == std::string::npos) break;
        std::string room = json.substr(q1 + 1, q2 - q1 - 1);
        std::string kind;
        auto kp = json.find("\"kind\"", q2);
        auto next_room = json.find("\"room_id\"", q2);
        if (kp != std::string::npos && (next_room == std::string::npos || kp < next_room)) {
            auto k1 = json.find('"', kp + 7); // 同上："kind" 长 7
            auto k2 = json.find('"', k1 + 1);
            if (k1 != std::string::npos && k2 != std::string::npos) kind = json.substr(k1 + 1, k2 - k1 - 1);
        }
        out.emplace_back(std::move(room), std::move(kind));
        pos = q2;
    }
    return out;
}

} // namespace viewer
