// mediaservo_control_relay — 舱端遥控 ↔ ROS2 桥接样例（G5 device-day 面）。
//
// 数据流：
//   ROS /mediaservo/cmd (std_msgs/String = JSON {label, cmd, payload})
//     → ms::Control::send(seq 自增) → 车端执行 → ack
//   ms::Control::on_ack → ROS /mediaservo/ack (std_msgs/String = ack JSON)
//
// 参数（ROS 声明，无硬编码）：http_base / ws_url / room / user / pass_param（口令走
// rosparam 或环境变量 MSRTC_PASS，不落命令行——PIT-171 纪律）。
//
// 构建（device-day，需 sdk-client 安装 + ROS2 humble）:
//   colcon build --packages-select mediaservo_control_relay
//   或独立: colcon build 于本目录（CMAKE_PREFIX_PATH 含 SDK 安装前缀）。

#include <atomic>
#include <chrono>
#include <cstdlib>
#include <memory>
#include <string>

#include <std_msgs/msg/string.hpp>
#include <rclcpp/rclcpp.hpp>

#include <mediaservo/client.hpp>

namespace ms = mediaservo::client;

class RelayNode : public rclcpp::Node {
public:
    RelayNode() : rclcpp::Node("mediaservo_control_relay") {
        http_base_ = declare_parameter<std::string>("http_base", "http://127.0.0.1:9800");
        ws_url_    = declare_parameter<std::string>("ws_url", "ws://127.0.0.1:9800/ws");
        room_      = declare_parameter<std::string>("room", "vehicle");
        user_      = declare_parameter<std::string>("user", "admin");

        ack_pub_ = create_publisher<std_msgs::msg::String>("/mediaservo/ack", 10);
        cmd_sub_ = create_subscription<std_msgs::msg::String>(
            "/mediaservo/cmd", 10,
            [this](const std_msgs::msg::String::SharedPtr msg) { on_cmd(msg); });

        spin_up();
    }

private:
    void spin_up() {
        // 口令仅接受 env（ROS param 明文落盘 = 凭证泄露面，拒绝）。
        const char* pass = std::getenv("MSRTC_PASS");
        if (pass == nullptr || *pass == '\0') {
            RCLCPP_ERROR(get_logger(), "MSRTC_PASS 未设置（口令只接受环境变量）");
            rclcpp::shutdown();
            return;
        }
        auto token = ms::login(http_base_, user_, pass);
        if (!token) {
            RCLCPP_ERROR(get_logger(), "login 失败: [%d] %s",
                         token.error().code, token.error().message.c_str());
            rclcpp::shutdown();
            return;
        }
        ms::Config cfg;
        cfg.signaling_url = ws_url_;
        cfg.room = room_;
        cfg.jwt = token.value();
        cfg.role = "Client";
        auto session = ms::Session::connect(cfg);
        if (!session) {
            RCLCPP_ERROR(get_logger(), "入房失败: [%d] %s",
                         session.error().code, session.error().message.c_str());
            rclcpp::shutdown();
            return;
        }
        auto ctl = session->open_control({"chassis", "gimbal", "light"});
        if (!ctl) {
            RCLCPP_ERROR(get_logger(), "open_control 失败: [%d] %s",
                         ctl.error().code, ctl.error().message.c_str());
            rclcpp::shutdown();
            return;
        }
        ctl->on_ack([this](const std::string& ack_json) {
            std_msgs::msg::String m;
            m.data = ack_json;
            ack_pub_->publish(m);
        });
        RCLCPP_INFO(get_logger(), "mediaservo 控制面就绪 room=%s negotiated=%u",
                    room_.c_str(), session->negotiated().value_or(0));
        session_.emplace(std::move(session.value()));
        ctl_.emplace(std::move(ctl.value()));
    }

    // cmd JSON 最小方言: {"label":"chassis","cmd":"steer","payload":{"deg":10}}
    // （payload 原样转发字符串拼接禁止——此处按 SDK 契约以 raw JSON 传递）。
    void on_cmd(const std_msgs::msg::String::SharedPtr& msg) {
        if (!ctl_) return;
        const std::string& raw = msg->data;
        auto field = [&raw](const char* key) -> std::string {
            const std::string k = std::string("\"") + key + "\":\"";
            auto p = raw.find(k);
            if (p == std::string::npos) return {};
            p += k.size();
            auto e = raw.find('"', p);
            return e == std::string::npos ? std::string{} : raw.substr(p, e - p);
        };
        const std::string label = field("label");
        const std::string cmd = field("cmd");
        auto pj = raw.find("\"payload\":");
        std::string payload;
        if (pj != std::string::npos) {
            pj += 10;
            auto st = raw.find_first_not_of(" \t", pj);
            if (st != std::string::npos && raw[st] == '{') {
                int depth = 0;
                auto en = st;
                for (; en < raw.size(); ++en) {
                    if (raw[en] == '{') ++depth;
                    else if (raw[en] == '}' && --depth == 0) break;
                }
                payload = raw.substr(st, en - st + 1);
            }
        }
        if (label.empty() || cmd.empty()) {
            RCLCPP_WARN(get_logger(), "cmd JSON 缺 label/cmd: %s", raw.c_str());
            return;
        }
        const uint64_t seq = next_seq_.fetch_add(1);
        auto sent = ctl_->send(label, seq, cmd, payload.empty() ? "{}" : payload);
        if (!sent) {
            RCLCPP_ERROR(get_logger(), "send 失败: [%d] %s",
                         sent.error().code, sent.error().message.c_str());
        }
    }

    std::string http_base_, ws_url_, room_, user_;
    rclcpp::Publisher<std_msgs::msg::String>::SharedPtr ack_pub_;
    rclcpp::Subscription<std_msgs::msg::String>::SharedPtr cmd_sub_;
    std::optional<ms::Session> session_;
    std::optional<ms::Control> ctl_;
    std::atomic<uint64_t> next_seq_{1};
};

int main(int argc, char** argv) {
    rclcpp::init(argc, argv);
    rclcpp::spin(std::make_shared<RelayNode>());
    rclcpp::shutdown();
    return 0;
}
