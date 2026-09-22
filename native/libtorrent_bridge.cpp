#include <libtorrent/magnet_uri.hpp>
#include <libtorrent/address.hpp>
#include <libtorrent/alert_types.hpp>
#include <libtorrent/create_torrent.hpp>
#include <libtorrent/read_resume_data.hpp>
#include <libtorrent/session.hpp>
#include <libtorrent/session_params.hpp>
#include <libtorrent/settings_pack.hpp>
#include <libtorrent/torrent_flags.hpp>
#include <libtorrent/torrent_info.hpp>
#include <libtorrent/write_resume_data.hpp>

#include <algorithm>
#include <cctype>
#include <chrono>
#include <cstdint>
#include <cstring>
#include <deque>
#include <exception>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <sstream>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

#include <fcntl.h>
#include <unistd.h>

namespace lt = libtorrent;

struct rextto_lt_event {
    int kind;
    char hash[65];
    char name[512];
    char save_path[1024];
};

struct rextto_lt_peer {
    char address[96];
    char client[256];
    int download_rate;
    int upload_rate;
    int num_pieces;
    int seed;
};

struct rextto_lt_tracker {
    char url[640];
    int tier;
    int status;
};

struct rextto_lt_file {
    char path[1024];
    long long size;
    long long downloaded;
};

struct rextto_lt_session {
    explicit rextto_lt_session(unsigned short port_min, unsigned short port_max,
        int download_limit, int upload_limit, int active_downloads, int active_seeds,
        int active_limit, int connections_limit, bool dht, bool pex, bool lsd, bool upnp, bool natpmp)
        : pex_enabled(pex), session(make_params(port_min, port_max, download_limit, upload_limit,
            active_downloads, active_seeds, active_limit, connections_limit, dht, lsd, upnp, natpmp)) {}

    static lt::session_params make_params(unsigned short port_min, unsigned short port_max,
        int download_limit, int upload_limit, int active_downloads, int active_seeds,
        int active_limit, int connections_limit, bool dht, bool lsd, bool upnp, bool natpmp) {
        lt::settings_pack settings;
        settings.set_str(lt::settings_pack::listen_interfaces,
            "0.0.0.0:" + std::to_string(port_min) + "-" + std::to_string(port_max));
        settings.set_int(lt::settings_pack::download_rate_limit, download_limit);
        settings.set_int(lt::settings_pack::upload_rate_limit, upload_limit);
        settings.set_int(lt::settings_pack::active_downloads, active_downloads);
        settings.set_int(lt::settings_pack::active_seeds, active_seeds);
        settings.set_int(lt::settings_pack::active_limit, active_limit);
        settings.set_int(lt::settings_pack::connections_limit, connections_limit);
        settings.set_bool(lt::settings_pack::enable_dht, dht);
        settings.set_bool(lt::settings_pack::enable_lsd, lsd);
        settings.set_bool(lt::settings_pack::enable_upnp, upnp);
        settings.set_bool(lt::settings_pack::enable_natpmp, natpmp);
        return lt::session_params(std::move(settings));
    }

    bool pex_enabled;
    bool sequential_enabled = false;
    lt::session session;
    std::deque<rextto_lt_event> events;
    std::unordered_map<std::string, std::chrono::steady_clock::time_point> active_since;
    std::vector<std::string> last_queue_order;
    std::deque<std::pair<std::chrono::steady_clock::time_point, std::int64_t>> dynamic_rate_samples;
    std::chrono::steady_clock::time_point dynamic_last_change =
        std::chrono::steady_clock::now() - std::chrono::seconds(600);
    std::chrono::steady_clock::time_point dynamic_seed_state_since = std::chrono::steady_clock::now();
    bool dynamic_seed_state = false;
    bool dynamic_was_enabled = false;
    int dynamic_saturation = 0;
    int dynamic_underused = 0;
};

struct rextto_lt_status {
    char hash[65];
    char name[512];
    char save_path[1024];
    double progress;
    int state;
    int paused;
    int download_rate;
    int upload_rate;
    int num_peers;
    int num_seeds;
    int download_limit;
    int upload_limit;
    long long all_time_upload;
    long long all_time_download;
    long long seeding_seconds;
    int queue_position;
    int has_metadata;
    int auto_managed;
    int torrent_version;
    long long total_size;
    long long total_done;
};

static void set_error(char* output, size_t output_size, const std::string& message) {
    if (output == nullptr || output_size == 0) return;
    std::strncpy(output, message.c_str(), output_size - 1);
    output[output_size - 1] = '\0';
}

static void copy_string(char* output, size_t output_size, const std::string& value) {
    if (output == nullptr || output_size == 0) return;
    std::strncpy(output, value.c_str(), output_size - 1);
    output[output_size - 1] = '\0';
}

static std::string hex_hash(lt::torrent_handle const& handle) {
    auto raw = handle.info_hash().to_string();
    static constexpr char hex[] = "0123456789abcdef";
    std::string result;
    result.reserve(raw.size() * 2);
    for (unsigned char byte : raw) {
        result.push_back(hex[(byte >> 4) & 0x0f]);
        result.push_back(hex[byte & 0x0f]);
    }
    return result;
}

static lt::torrent_handle find_torrent(rextto_lt_session* session, const char* hash) {
    if (session == nullptr || hash == nullptr) return {};
    for (auto const& handle : session->session.get_torrents()) {
        if (hex_hash(handle) == hash) return handle;
    }
    return {};
}

static void collect_events(rextto_lt_session* session) {
    std::vector<lt::alert*> alerts;
    session->session.pop_alerts(&alerts);
    for (auto* alert : alerts) {
        int kind = 0;
        lt::torrent_handle handle;
        const char* moved_path = nullptr;
        if (const auto* event = lt::alert_cast<lt::metadata_received_alert>(alert)) { kind = 1; handle = event->handle; handle.set_flags(lt::torrent_flags::auto_managed); }
        else if (const auto* event = lt::alert_cast<lt::torrent_finished_alert>(alert)) { kind = 2; handle = event->handle; }
        else if (const auto* event = lt::alert_cast<lt::storage_moved_alert>(alert)) { kind = 3; handle = event->handle; moved_path = event->storage_path(); }
        if (kind == 0 || !handle.is_valid()) continue;
        auto status = handle.status(lt::torrent_handle::query_name | lt::torrent_handle::query_save_path);
        rextto_lt_event output{};
        output.kind = kind;
        copy_string(output.hash, sizeof(output.hash), hex_hash(handle));
        copy_string(output.name, sizeof(output.name), status.name);
        copy_string(output.save_path, sizeof(output.save_path), moved_path == nullptr ? status.save_path : moved_path);
        session->events.push_back(output);
    }
}

static bool write_atomic(const std::filesystem::path& path, const std::vector<char>& content, std::string& error) {
    const auto temporary = path.string() + ".tmp";
    const int fd = ::open(temporary.c_str(), O_WRONLY | O_CREAT | O_TRUNC, 0600);
    if (fd < 0) { error = std::strerror(errno); return false; }
    size_t written = 0;
    while (written < content.size()) {
        const auto result = ::write(fd, content.data() + written, content.size() - written);
        if (result < 0) { error = std::strerror(errno); ::close(fd); ::unlink(temporary.c_str()); return false; }
        written += static_cast<size_t>(result);
    }
    if (::fsync(fd) != 0) { error = std::strerror(errno); ::close(fd); ::unlink(temporary.c_str()); return false; }
    if (::close(fd) != 0) { error = std::strerror(errno); ::unlink(temporary.c_str()); return false; }
    if (::rename(temporary.c_str(), path.c_str()) != 0) { error = std::strerror(errno); ::unlink(temporary.c_str()); return false; }
    const int dir = ::open(path.parent_path().c_str(), O_RDONLY | O_DIRECTORY);
    if (dir >= 0) { ::fsync(dir); ::close(dir); }
    return true;
}

static int load_ipfilter_into_session(rextto_lt_session* session, const std::string& path) {
    if (session == nullptr) return 0;
    std::ifstream filter_file(path);
    if (!filter_file) return 0;
    lt::ip_filter filter;
    std::string rule_line;
    int rules = 0;
    auto trim = [](std::string text) {
        auto not_space = [](unsigned char character) { return !std::isspace(character); };
        text.erase(text.begin(), std::find_if(text.begin(), text.end(), not_space));
        text.erase(std::find_if(text.rbegin(), text.rend(), not_space).base(), text.end());
        return text;
    };
    while (std::getline(filter_file, rule_line)) {
        auto comment = rule_line.find('#');
        if (comment != std::string::npos) rule_line.erase(comment);
        auto colon = rule_line.find_last_of(':');
        std::string range = colon == std::string::npos ? rule_line : rule_line.substr(colon + 1);
        auto dash = range.find('-');
        if (dash == std::string::npos) continue;
        try {
            auto first = lt::make_address(trim(range.substr(0, dash)));
            auto last = lt::make_address(trim(range.substr(dash + 1)));
            filter.add_rule(first, last, lt::ip_filter::blocked);
            ++rules;
        } catch (...) {
        }
    }
    if (rules > 0) {
        session->session.set_ip_filter(filter);
    }
    return rules;
}

extern "C" {
rextto_lt_session* rextto_lt_create(unsigned short port_min, unsigned short port_max,    int download_limit, int upload_limit, int active_downloads, int active_seeds,
    int active_limit, int connections_limit, unsigned char dht, unsigned char pex, unsigned char lsd,
    unsigned char upnp, unsigned char natpmp, char* error, size_t error_size) {
    try {
        return new rextto_lt_session(port_min, port_max, download_limit, upload_limit,
            active_downloads, active_seeds, active_limit, connections_limit,
            dht != 0, pex != 0, lsd != 0, upnp != 0, natpmp != 0);
    } catch (const std::exception& exception) {
        set_error(error, error_size, exception.what());
    } catch (...) {
        set_error(error, error_size, "unknown libtorrent session error");
    }
    return nullptr;
}

void rextto_lt_destroy(rextto_lt_session* session) {
    delete session;
}

void rextto_lt_version(char* output, size_t output_size) {
    copy_string(output, output_size, lt::version());
}

int rextto_lt_load_ipfilter(rextto_lt_session* session, const char* path, int* rules_out, char* error, size_t error_size) {
    if (session == nullptr || path == nullptr) {
        set_error(error, error_size, "invalid ip filter parameters");
        return 0;
    }
    try {
        int rules = load_ipfilter_into_session(session, std::string(path));
        if (rules_out != nullptr) *rules_out = rules;
        return 1;
    } catch (const std::exception& exception) {
        set_error(error, error_size, exception.what());
    } catch (...) {
        set_error(error, error_size, "unknown ip filter error");
    }
    return 0;
}

int rextto_lt_save_torrent(rextto_lt_session* session, const char* hash, const char* path, char* error, size_t error_size) {
    if (session == nullptr || hash == nullptr || path == nullptr) {
        set_error(error, error_size, "invalid torrent metadata save parameters");
        return 0;
    }
    try {
        auto handle = find_torrent(session, hash);
        if (!handle.is_valid()) { set_error(error, error_size, "torrent not found"); return 0; }
        auto info = handle.torrent_file();
        if (!info) { set_error(error, error_size, "torrent metadata unavailable"); return 0; }
        lt::create_torrent creator(*info);
        std::vector<char> content;
        lt::bencode(std::back_inserter(content), creator.generate());
        const std::filesystem::path target(path);
        std::filesystem::create_directories(target.parent_path());
        std::string write_error;
        if (!write_atomic(target, content, write_error)) {
            set_error(error, error_size, write_error);
            return 0;
        }
        return 1;
    } catch (const std::exception& exception) {
        set_error(error, error_size, exception.what());
    } catch (...) {
        set_error(error, error_size, "unknown torrent metadata save error");
    }
    return 0;
}

int rextto_lt_apply_settings(rextto_lt_session* session, const char* settings, char* error, size_t error_size) {
    if (session == nullptr || settings == nullptr) {
        set_error(error, error_size, "invalid libtorrent settings parameters");
        return 0;
    }
    try {
        lt::settings_pack pack;
        std::istringstream stream(settings);
        std::string line;
        int applied = 0;
        while (std::getline(stream, line)) {
            if (line.size() < 3 || line[1] != ':') continue;
            char type = line[0];
            auto separator = line.find('=', 2);
            if (separator == std::string::npos) continue;
            std::string key = line.substr(2, separator - 2);
            std::string value = line.substr(separator + 1);
            bool known = false;
            int number = 0;
            bool flag = value == "1" || value == "true" || value == "yes";
            if (type == 'i') {
                try { number = static_cast<int>(std::stoll(value)); } catch (...) { continue; }
            }
            if (key == "connections_limit") { pack.set_int(lt::settings_pack::connections_limit, number); known = true; }
            else if (key == "half_open_limit") { pack.set_int(lt::settings_pack::half_open_limit, number); known = true; }
            else if (key == "alert_queue_size") { pack.set_int(lt::settings_pack::alert_queue_size, number); known = true; }
            else if (key == "aio_threads") { pack.set_int(lt::settings_pack::aio_threads, number); known = true; }
            else if (key == "cache_size") { pack.set_int(lt::settings_pack::cache_size, number); known = true; }
            else if (key == "cache_expiry") { pack.set_int(lt::settings_pack::cache_expiry, number); known = true; }
            else if (key == "download_rate_limit") { pack.set_int(lt::settings_pack::download_rate_limit, number); known = true; }
            else if (key == "upload_rate_limit") { pack.set_int(lt::settings_pack::upload_rate_limit, number); known = true; }
            else if (key == "announce_interval") { pack.set_int(lt::settings_pack::min_announce_interval, number); known = true; }
            else if (key == "torrent_connect_boost") { pack.set_int(lt::settings_pack::torrent_connect_boost, number); known = true; }
            else if (key == "in_enc_policy") { pack.set_int(lt::settings_pack::in_enc_policy, number); known = true; }
            else if (key == "out_enc_policy") { pack.set_int(lt::settings_pack::out_enc_policy, number); known = true; }
            else if (key == "proxy_type") { pack.set_int(lt::settings_pack::proxy_type, number); known = true; }
            else if (key == "proxy_port") { pack.set_int(lt::settings_pack::proxy_port, number); known = true; }
            else if (key == "enable_utp") { pack.set_bool(lt::settings_pack::enable_outgoing_utp, flag); pack.set_bool(lt::settings_pack::enable_incoming_utp, flag); known = true; }
            else if (key == "prefer_rc4") { pack.set_bool(lt::settings_pack::prefer_rc4, flag); known = true; }
            else if (key == "announce_to_all_trackers") { pack.set_bool(lt::settings_pack::announce_to_all_trackers, flag); known = true; }
            else if (key == "announce_to_all_tiers") { pack.set_bool(lt::settings_pack::announce_to_all_tiers, flag); known = true; }
            else if (key == "allow_multiple_connections_per_ip") { pack.set_bool(lt::settings_pack::allow_multiple_connections_per_ip, flag); known = true; }
            else if (key == "apply_ip_filter") { pack.set_bool(lt::settings_pack::apply_ip_filter_to_trackers, flag); known = true; }
            else if (key == "proxy_host") { pack.set_str(lt::settings_pack::proxy_hostname, value); known = true; }
            else if (key == "proxy_username") { pack.set_str(lt::settings_pack::proxy_username, value); known = true; }
            else if (key == "proxy_password") { pack.set_str(lt::settings_pack::proxy_password, value); known = true; }
            else if (key == "listen_interfaces") { pack.set_str(lt::settings_pack::listen_interfaces, value); known = true; }
            // Killswitch VPN: forza il traffico in uscita su una scheda (estensione legacy).
            else if (key == "outgoing_interfaces") { pack.set_str(lt::settings_pack::outgoing_interfaces, value); known = true; }
            else if (key == "ip_filter_path") { load_ipfilter_into_session(session, value); known = true; }
            else if (key == "dht_bootstrap_nodes") { pack.set_str(lt::settings_pack::dht_bootstrap_nodes, value); known = true; }
            else if (key == "max_peerlist_size") { pack.set_int(lt::settings_pack::max_peerlist_size, number); known = true; }
            else if (key == "max_out_request_queue") { pack.set_int(lt::settings_pack::max_out_request_queue, number); known = true; }
            else if (key == "max_allowed_in_request_queue") { pack.set_int(lt::settings_pack::max_allowed_in_request_queue, number); known = true; }
            else if (key == "whole_pieces_threshold") { pack.set_int(lt::settings_pack::whole_pieces_threshold, number); known = true; }
            else if (key == "request_queue_time") { pack.set_int(lt::settings_pack::request_queue_time, number); known = true; }
            else if (key == "peer_connect_timeout") { pack.set_int(lt::settings_pack::peer_connect_timeout, number); known = true; }
            else if (key == "request_timeout") { pack.set_int(lt::settings_pack::request_timeout, number); known = true; }
            else if (key == "max_queued_disk_bytes") { pack.set_int(lt::settings_pack::max_queued_disk_bytes, number); known = true; }
            else if (key == "send_buffer_watermark") { pack.set_int(lt::settings_pack::send_buffer_watermark, number); known = true; }
            else if (key == "send_buffer_low_watermark") { pack.set_int(lt::settings_pack::send_buffer_low_watermark, number); known = true; }
            else if (key == "send_buffer_watermark_factor") { pack.set_int(lt::settings_pack::send_buffer_watermark_factor, number); known = true; }
            else if (key == "send_socket_buffer_size") { pack.set_int(lt::settings_pack::send_socket_buffer_size, number); known = true; }
            else if (key == "recv_socket_buffer_size") { pack.set_int(lt::settings_pack::recv_socket_buffer_size, number); known = true; }
            else if (key == "file_pool_size") { pack.set_int(lt::settings_pack::file_pool_size, number); known = true; }
            else if (key == "checking_mem_usage") { pack.set_int(lt::settings_pack::checking_mem_usage, number); known = true; }
            else if (key == "max_rejects") { pack.set_int(lt::settings_pack::max_rejects, number); known = true; }
            else if (key == "mixed_mode_algorithm") { pack.set_int(lt::settings_pack::mixed_mode_algorithm, number); known = true; }
            else if (key == "active_downloads") { pack.set_int(lt::settings_pack::active_downloads, number); known = true; }
            else if (key == "active_seeds") { pack.set_int(lt::settings_pack::active_seeds, number); known = true; }
            else if (key == "active_limit") { pack.set_int(lt::settings_pack::active_limit, number); known = true; }
            else if (key == "active_checking") { pack.set_int(lt::settings_pack::active_checking, number); known = true; }
            else if (key == "auto_manage_interval") { pack.set_int(lt::settings_pack::auto_manage_interval, number); known = true; }
            else if (key == "smooth_connects") { pack.set_bool(lt::settings_pack::smooth_connects, flag); known = true; }
            else if (key == "strict_end_game_mode") { pack.set_bool(lt::settings_pack::strict_end_game_mode, flag); known = true; }
            else if (key == "dont_count_slow_torrents") { pack.set_bool(lt::settings_pack::dont_count_slow_torrents, flag); known = true; }
            else if (key == "proxy_peer_connections") { pack.set_bool(lt::settings_pack::proxy_peer_connections, flag); known = true; }
            if (known) ++applied;
        }
        if (applied == 0) return 0;
        session->session.apply_settings(pack);
        return applied;
    } catch (const std::exception& exception) {
        set_error(error, error_size, exception.what());
    } catch (...) {
        set_error(error, error_size, "unknown libtorrent settings error");
    }
    return 0;
}

int rextto_lt_add(rextto_lt_session* session, const char* magnet, const char* save_path, char* error, size_t error_size) {
    if (session == nullptr || magnet == nullptr || save_path == nullptr) {
        set_error(error, error_size, "invalid libtorrent add parameters");
        return 0;
    }
    try {
        lt::error_code ec;
        lt::add_torrent_params params = lt::parse_magnet_uri(magnet, ec);
        if (ec) {
            set_error(error, error_size, ec.message());
            return 0;
        }
        params.save_path = save_path;
        if (!session->pex_enabled) params.flags |= lt::torrent_flags::disable_pex;
        if (session->sequential_enabled) params.flags |= lt::torrent_flags::sequential_download;
        session->session.add_torrent(std::move(params), ec);
        if (ec) {
            set_error(error, error_size, ec.message());
            return 0;
        }
        return 1;
    } catch (const std::exception& exception) {
        set_error(error, error_size, exception.what());
    } catch (...) {
        set_error(error, error_size, "unknown libtorrent add error");
    }
    return 0;
}

int rextto_lt_add_file(rextto_lt_session* session, const char* torrent_path, const char* save_path, char* hash, size_t hash_size, char* error, size_t error_size) {
    if (session == nullptr || torrent_path == nullptr || save_path == nullptr) {
        set_error(error, error_size, "invalid torrent-file add parameters");
        return 0;
    }
    try {
        lt::error_code ec;
        lt::add_torrent_params params;
        params.ti = std::make_shared<lt::torrent_info>(std::string(torrent_path), ec);
        if (ec) { set_error(error, error_size, ec.message()); return 0; }
        params.save_path = save_path;
        if (!session->pex_enabled) params.flags |= lt::torrent_flags::disable_pex;
        if (session->sequential_enabled) params.flags |= lt::torrent_flags::sequential_download;
        auto handle = session->session.add_torrent(std::move(params), ec);
        if (ec) { set_error(error, error_size, ec.message()); return 0; }
        copy_string(hash, hash_size, hex_hash(handle));
        return 1;
    } catch (const std::exception& exception) {
        set_error(error, error_size, exception.what());
    } catch (...) {
        set_error(error, error_size, "unknown torrent-file add error");
    }
    return 0;
}

unsigned int rextto_lt_torrent_count(const rextto_lt_session* session) {
    if (session == nullptr) return 0;
    return static_cast<unsigned int>(session->session.get_torrents().size());
}

size_t rextto_lt_statuses(const rextto_lt_session* session, rextto_lt_status* output, size_t capacity) {
    if (session == nullptr) return 0;
    auto statuses = session->session.get_torrent_status(
        [](lt::torrent_status const&) { return true; },
        lt::torrent_handle::query_name | lt::torrent_handle::query_save_path);
    if (output == nullptr || capacity == 0) return statuses.size();
    const size_t count = std::min(capacity, statuses.size());
    for (size_t i = 0; i < count; ++i) {
        auto const& status = statuses[i];
        output[i] = {};
        copy_string(output[i].hash, sizeof(output[i].hash), hex_hash(status.handle));
        copy_string(output[i].name, sizeof(output[i].name), status.name);
        copy_string(output[i].save_path, sizeof(output[i].save_path), status.save_path);
        output[i].progress = static_cast<double>(status.progress) * 100.0;
        output[i].state = static_cast<int>(status.state);
        output[i].paused = static_cast<bool>(status.flags & lt::torrent_flags::paused) ? 1 : 0;
        output[i].download_rate = status.download_rate;
        output[i].upload_rate = status.upload_rate;
        output[i].num_peers = status.num_peers;
        output[i].num_seeds = status.num_seeds;
        output[i].download_limit = status.handle.download_limit();
        output[i].upload_limit = status.handle.upload_limit();
        output[i].all_time_upload = status.all_time_upload;
        output[i].all_time_download = status.all_time_download;
        output[i].seeding_seconds = status.seeding_duration.count();
        output[i].queue_position = static_cast<int>(status.queue_position);
        output[i].has_metadata = status.has_metadata ? 1 : 0;
        output[i].auto_managed = static_cast<bool>(status.flags & lt::torrent_flags::auto_managed) ? 1 : 0;
        output[i].torrent_version = 0;
        output[i].total_size = status.total_wanted;
        output[i].total_done = status.total_wanted_done;
        if (status.has_metadata) {
#if LIBTORRENT_VERSION_NUM >= 20000
            auto const& hashes = status.info_hashes;
            if (hashes.has_v1() && hashes.has_v2()) output[i].torrent_version = 3;
            else if (hashes.has_v2()) output[i].torrent_version = 2;
            else if (hashes.has_v1()) output[i].torrent_version = 1;
#else
            output[i].torrent_version = 1;
#endif
        }
    }
    return count;
}

size_t rextto_lt_events(rextto_lt_session* session, rextto_lt_event* output, size_t capacity) {
    if (session == nullptr) return 0;
    collect_events(session);
    if (output == nullptr || capacity == 0) return 0;
    const size_t count = std::min(capacity, session->events.size());
    for (size_t i = 0; i < count; ++i) {
        output[i] = session->events.front();
        session->events.pop_front();
    }
    return count;
}

void rextto_lt_promote_metadata(rextto_lt_session* session) {
    if (session == nullptr) return;
    const auto now = std::chrono::steady_clock::now();
    for (auto const& handle : session->session.get_torrents()) {
        auto status = handle.status();
        const auto hash = hex_hash(handle);
        if (!status.has_metadata && (status.flags & lt::torrent_flags::auto_managed) && (status.flags & lt::torrent_flags::paused)) {
            handle.unset_flags(lt::torrent_flags::auto_managed);
            handle.resume();
            session->active_since[hash] = now;
        }
    }
}

void rextto_lt_prioritize_queue(rextto_lt_session* session) {
    if (session == nullptr) return;
    struct candidate { lt::torrent_handle handle; std::string hash; double score; };
    const auto now = std::chrono::steady_clock::now();
    std::vector<candidate> candidates;
    std::vector<std::string> protected_order;
    for (auto const& handle : session->session.get_torrents()) {
        auto status = handle.status();
        const auto hash = hex_hash(handle);
        if (status.is_seeding || status.is_finished || !status.has_metadata) continue;
        if (!(status.flags & lt::torrent_flags::paused)) session->active_since.try_emplace(hash, now);
        const auto active = session->active_since.find(hash);
        if (active != session->active_since.end() && now - active->second < std::chrono::seconds(600)) {
            protected_order.push_back(hash);
            continue;
        }
        const double remaining = static_cast<double>(std::max<std::int64_t>(0, status.total_wanted - status.total_wanted_done));
        const int sources = status.num_peers + status.num_seeds;
        const double score = status.download_rate > 0 ? remaining / status.download_rate : (sources > 0 ? remaining / (1.0 + sources) : remaining);
        candidates.push_back({handle, hash, score});
    }
    // Lower badness means a lower ETA / better source availability.  legacy
    // therefore puts the smallest score first; sorting in reverse starved
    // torrents that were already close to completion.
    std::sort(candidates.begin(), candidates.end(), [](candidate const& left, candidate const& right) { return left.score < right.score; });
    std::vector<std::string> order = protected_order;
    for (auto const& item : candidates) order.push_back(item.hash);
    if (order == session->last_queue_order) return;
    session->last_queue_order = order;
    for (auto const& item : candidates) item.handle.queue_position_top();
    for (auto const& hash : protected_order) {
        auto handle = find_torrent(session, hash.c_str());
        if (handle.is_valid()) handle.queue_position_top();
    }
}

void rextto_lt_adjust_queue(rextto_lt_session* session, int enabled, int static_downloads, int minimum, int maximum, int static_seeds, int static_limit, int global_download_limit) {
    if (session == nullptr) return;
    minimum = std::max(1, minimum);
    maximum = std::max(minimum, maximum);
    auto settings = session->session.get_settings();

    // Dynamic values belong to the live session, not to the configuration DB.
    // Disabling the feature must therefore restore the configured baseline and
    // remove the per-torrent overrides installed by the dynamic allocator.
    if (!enabled) {
        settings.set_int(lt::settings_pack::active_downloads, std::max(1, static_downloads));
        settings.set_int(lt::settings_pack::active_seeds, std::max(1, static_seeds));
        settings.set_int(lt::settings_pack::active_limit, std::max(1, static_limit));
        session->session.apply_settings(settings);
        if (session->dynamic_was_enabled) {
            for (auto const& handle : session->session.get_torrents()) {
                handle.set_max_connections(-1);
                handle.set_max_uploads(-1);
                handle.set_download_limit(0);
            }
        }
        session->dynamic_rate_samples.clear();
        session->dynamic_saturation = 0;
        session->dynamic_underused = 0;
        session->dynamic_was_enabled = false;
        return;
    }
    session->dynamic_was_enabled = true;

    const auto now = std::chrono::steady_clock::now();
    int downloads = settings.get_int(lt::settings_pack::active_downloads);
    int seeds = settings.get_int(lt::settings_pack::active_seeds);
    int active_limit = settings.get_int(lt::settings_pack::active_limit);
    std::int64_t aggregate_rate = 0;
    int queued_count = 0;
    int queued_with_sources = 0;
    bool priority_download = false;
    struct active_torrent { lt::torrent_handle handle; int tier; };
    std::vector<active_torrent> active;
    for (auto const& handle : session->session.get_torrents()) {
        auto status = handle.status();
        if (!status.has_metadata || status.is_seeding || status.is_finished) continue;
        const bool paused = static_cast<bool>(status.flags & lt::torrent_flags::paused);
        const int sources = status.num_peers + status.num_seeds;
        if (paused) {
            ++queued_count;
            if (sources > 0) ++queued_with_sources;
            continue;
        }
        const int tier = status.download_rate > 0 ? 0 : (sources > 0 ? 1 : 2);
        aggregate_rate += std::max(0, status.download_rate);
        priority_download = priority_download || tier == 0;
        active.push_back({handle, tier});
    }

    // Moving-window hysteresis, matching legacy: three coherent samples and a
    // ten-minute cooldown prevent queue oscillation on a fluctuating link.
    session->dynamic_rate_samples.emplace_back(now, aggregate_rate);
    while (!session->dynamic_rate_samples.empty()
        && now - session->dynamic_rate_samples.front().first > std::chrono::seconds(600)) {
        session->dynamic_rate_samples.pop_front();
    }
    if (now - session->dynamic_last_change >= std::chrono::seconds(600)) {
        if (global_download_limit > 0 && session->dynamic_rate_samples.size() >= 3) {
            std::int64_t sum = 0;
            for (auto const& sample : session->dynamic_rate_samples) sum += sample.second;
            const double ratio = static_cast<double>(sum) / session->dynamic_rate_samples.size() / global_download_limit;
            if (ratio >= 0.9 && downloads > minimum) {
                ++session->dynamic_saturation;
                session->dynamic_underused = 0;
                if (session->dynamic_saturation >= 3) {
                    downloads = std::max(minimum, downloads - 1);
                    session->dynamic_saturation = 0;
                    session->dynamic_last_change = now;
                }
            } else if (queued_with_sources > 0 && ratio <= 0.5 && downloads < maximum) {
                ++session->dynamic_underused;
                session->dynamic_saturation = 0;
                if (session->dynamic_underused >= 3) {
                    downloads = std::min(maximum, downloads + 1);
                    session->dynamic_underused = 0;
                    session->dynamic_last_change = now;
                }
            } else {
                session->dynamic_saturation = 0;
                session->dynamic_underused = 0;
            }
        } else if (global_download_limit <= 0 && queued_with_sources > 0 && downloads < maximum) {
            ++session->dynamic_underused;
            if (session->dynamic_underused >= 3) {
                downloads = std::min(maximum, downloads + 1);
                session->dynamic_underused = 0;
                session->dynamic_last_change = now;
            }
        } else {
            session->dynamic_saturation = 0;
            session->dynamic_underused = 0;
        }
    }

    const bool seed_priority_state = queued_count > 0;
    if (seed_priority_state != session->dynamic_seed_state) {
        session->dynamic_seed_state = seed_priority_state;
        session->dynamic_seed_state_since = now;
    } else if (now - session->dynamic_seed_state_since >= std::chrono::seconds(300)) {
        seeds = seed_priority_state ? 1 : std::max(1, static_seeds);
    }
    int desired_limit = std::max(static_limit, downloads + seeds + 2);
    if (downloads >= settings.get_int(lt::settings_pack::active_downloads)
        && seeds >= settings.get_int(lt::settings_pack::active_seeds)) {
        desired_limit = std::max(active_limit, desired_limit);
    }
    settings.set_int(lt::settings_pack::active_downloads, downloads);
    settings.set_int(lt::settings_pack::active_seeds, seeds);
    settings.set_int(lt::settings_pack::active_limit, desired_limit);
    session->session.apply_settings(settings);

    // Give more of the global pool to torrents that are already transferring
    // or have sources ready, and keep seeding from starving a live download.
    if (!active.empty()) {
        const int connection_limit = std::max(1, settings.get_int(lt::settings_pack::connections_limit));
        const int upload_limit = std::max(1, settings.get_int(lt::settings_pack::unchoke_slots_limit));
        int total_weight = 0;
        for (auto const& item : active) {
            total_weight += item.tier == 0 ? 3 : (item.tier == 1 ? 2 : 1);
        }
        const int available_connections = std::max(static_cast<int>(connection_limit * 0.7), 10 * static_cast<int>(active.size()));
        const int available_uploads = std::max(static_cast<int>(upload_limit * 0.7), 2 * static_cast<int>(active.size()));
        for (auto const& item : active) {
            const int weight = item.tier == 0 ? 3 : (item.tier == 1 ? 2 : 1);
            item.handle.set_max_connections(std::max(10, available_connections * weight / std::max(1, total_weight)));
            item.handle.set_max_uploads(std::max(2, available_uploads * weight / std::max(1, total_weight)));
            // Non imporre un tetto per-torrent basato sul numero di download
            // attivi: con molti torrent che scambiano pochi KB/s il pool globale
            // veniva diviso fra tutti, strozzando i torrent veloci e lasciando
            // inutilizzata parte della banda. Il tetto globale della sessione
            // (download_rate_limit) limita già il totale, quindi il singolo
            // torrent resta illimitato (0). Questo azzera anche eventuali limiti
            // per-torrent installati dalla versione precedente.
            item.handle.set_download_limit(0);
        }
        for (auto const& handle : session->session.get_torrents()) {
            auto status = handle.status();
            if (status.is_seeding || status.is_finished) handle.set_max_uploads(priority_download ? 2 : -1);
        }
    }
}

size_t rextto_lt_peers(rextto_lt_session* session, const char* hash, rextto_lt_peer* output, size_t capacity, char* error, size_t error_size) {
    try {
        auto handle = find_torrent(session, hash);
        if (!handle.is_valid()) { set_error(error, error_size, "torrent not found"); return 0; }
        std::vector<lt::peer_info> peers;
        handle.get_peer_info(peers);
        if (output == nullptr || capacity == 0) return peers.size();
        const size_t count = std::min(capacity, peers.size());
        for (size_t i = 0; i < count; ++i) {
            output[i] = {};
            copy_string(output[i].address, sizeof(output[i].address), peers[i].ip.address().to_string() + ":" + std::to_string(peers[i].ip.port()));
            copy_string(output[i].client, sizeof(output[i].client), peers[i].client);
            output[i].download_rate = peers[i].payload_down_speed;
            output[i].upload_rate = peers[i].payload_up_speed;
            output[i].num_pieces = peers[i].num_pieces;
            output[i].seed = static_cast<bool>(peers[i].flags & lt::peer_info::seed) ? 1 : 0;
        }
        return count;
    } catch (const std::exception& exception) {
        set_error(error, error_size, exception.what());
    }
    return 0;
}

size_t rextto_lt_trackers(rextto_lt_session* session, const char* hash, rextto_lt_tracker* output, size_t capacity, char* error, size_t error_size) {
    try {
        auto handle = find_torrent(session, hash);
        if (!handle.is_valid()) { set_error(error, error_size, "torrent not found"); return 0; }
        auto trackers = handle.trackers();
        if (output == nullptr || capacity == 0) return trackers.size();
        const size_t count = std::min(capacity, trackers.size());
        for (size_t i = 0; i < count; ++i) {
            output[i] = {};
            copy_string(output[i].url, sizeof(output[i].url), trackers[i].url);
            output[i].tier = trackers[i].tier;
        }
        return count;
    } catch (const std::exception& exception) {
        set_error(error, error_size, exception.what());
    }
    return 0;
}

size_t rextto_lt_files(rextto_lt_session* session, const char* hash, rextto_lt_file* output, size_t capacity, char* error, size_t error_size) {
    try {
        auto handle = find_torrent(session, hash);
        if (!handle.is_valid()) { set_error(error, error_size, "torrent not found"); return 0; }
        auto info = handle.torrent_file();
        if (!info) return 0;
        auto storage = info->files();
        std::vector<std::int64_t> progress = handle.file_progress();
        const int total = storage.num_files();
        if (output == nullptr || capacity == 0) return static_cast<size_t>(total);
        const size_t count = std::min(capacity, static_cast<size_t>(total));
        for (size_t i = 0; i < count; ++i) {
            output[i] = {};
            copy_string(output[i].path, sizeof(output[i].path), storage.file_path(static_cast<int>(i)));
            output[i].size = storage.file_size(static_cast<int>(i));
            output[i].downloaded = i < progress.size() ? progress[i] : 0;
        }
        return count;
    } catch (const std::exception& exception) {
        set_error(error, error_size, exception.what());
    }
    return 0;
}

int rextto_lt_move_storage(rextto_lt_session* session, const char* hash, const char* destination, char* error, size_t error_size) {
    if (session == nullptr || hash == nullptr || destination == nullptr) {
        set_error(error, error_size, "invalid move storage parameters");
        return 0;
    }
    try {
        auto handle = find_torrent(session, hash);
        if (!handle.is_valid()) { set_error(error, error_size, "torrent not found"); return 0; }
        handle.move_storage(destination, lt::move_flags_t::fail_if_exist);
        return 1;
    } catch (const std::exception& exception) {
        set_error(error, error_size, exception.what());
    }
    return 0;
}

int rextto_lt_set_paused(rextto_lt_session* session, const char* hash, int paused, char* error, size_t error_size) {
    try {
        auto handle = find_torrent(session, hash);
        if (!handle.is_valid()) { set_error(error, error_size, "torrent not found"); return 0; }
        if (paused) {
            handle.unset_flags(lt::torrent_flags::auto_managed);
            handle.pause();
        } else {
            handle.set_flags(lt::torrent_flags::auto_managed);
            handle.resume();
        }
        return 1;
    } catch (const std::exception& exception) {
        set_error(error, error_size, exception.what());
    }
    return 0;
}

int rextto_lt_set_pin(rextto_lt_session* session, const char* hash, int pinned, char* error, size_t error_size) {
    try {
        auto handle = find_torrent(session, hash);
        if (!handle.is_valid()) { set_error(error, error_size, "torrent not found"); return 0; }
        if (pinned) {
            handle.unset_flags(lt::torrent_flags::auto_managed);
            handle.resume();
            handle.queue_position_top();
        } else {
            handle.set_flags(lt::torrent_flags::auto_managed);
            handle.resume();
        }
        return 1;
    } catch (const std::exception& exception) {
        set_error(error, error_size, exception.what());
    } catch (...) {
        set_error(error, error_size, "unknown libtorrent pin error");
    }
    return 0;
}

int rextto_lt_set_sequential(rextto_lt_session* session, int enabled, char* error, size_t error_size) {
    try {
        if (session == nullptr) { set_error(error, error_size, "invalid session"); return 0; }
        session->sequential_enabled = enabled != 0;
        for (auto const& handle : session->session.get_torrents()) {
            if (session->sequential_enabled) {
                handle.set_flags(lt::torrent_flags::sequential_download);
            } else {
                handle.unset_flags(lt::torrent_flags::sequential_download);
            }
        }
        return 1;
    } catch (const std::exception& exception) {
        set_error(error, error_size, exception.what());
    } catch (...) {
        set_error(error, error_size, "unknown libtorrent sequential error");
    }
    return 0;
}

int rextto_lt_remove(rextto_lt_session* session, const char* hash, int delete_files, char* error, size_t error_size) {
    try {
        auto handle = find_torrent(session, hash);
        if (!handle.is_valid()) { set_error(error, error_size, "torrent not found"); return 0; }
        session->session.remove_torrent(handle, delete_files ? lt::session_handle::delete_files : lt::remove_flags_t{});
        return 1;
    } catch (const std::exception& exception) {
        set_error(error, error_size, exception.what());
    }
    return 0;
}

int rextto_lt_force_recheck(rextto_lt_session* session, const char* hash, char* error, size_t error_size) {
    try {
        auto handle = find_torrent(session, hash);
        if (!handle.is_valid()) { set_error(error, error_size, "torrent not found"); return 0; }
        handle.force_recheck();
        return 1;
    } catch (const std::exception& exception) {
        set_error(error, error_size, exception.what());
    }
    return 0;
}

int rextto_lt_reannounce(rextto_lt_session* session, const char* hash, char* error, size_t error_size) {
    try {
        auto handle = find_torrent(session, hash);
        if (!handle.is_valid()) { set_error(error, error_size, "torrent not found"); return 0; }
        handle.force_reannounce();
        return 1;
    } catch (const std::exception& exception) {
        set_error(error, error_size, exception.what());
    }
    return 0;
}

int rextto_lt_set_limits(rextto_lt_session* session, const char* hash, int download_limit, int upload_limit, char* error, size_t error_size) {
    try {
        auto handle = find_torrent(session, hash);
        if (!handle.is_valid()) { set_error(error, error_size, "torrent not found"); return 0; }
        handle.set_download_limit(download_limit);
        handle.set_upload_limit(upload_limit);
        return 1;
    } catch (const std::exception& exception) {
        set_error(error, error_size, exception.what());
    }
    return 0;
}

size_t rextto_lt_restore(rextto_lt_session* session, const char* state_dir, char* error, size_t error_size) {
    if (session == nullptr || state_dir == nullptr) { set_error(error, error_size, "invalid resume restore parameters"); return 0; }
    try {
        const std::filesystem::path directory(state_dir);
        if (!std::filesystem::is_directory(directory)) return 0;
        size_t restored = 0;
        std::string warning;
        for (const auto& entry : std::filesystem::directory_iterator(directory)) {
            if (!entry.is_regular_file() || entry.path().extension() != ".fastresume") continue;
            std::ifstream input(entry.path(), std::ios::binary);
            std::vector<char> content((std::istreambuf_iterator<char>(input)), std::istreambuf_iterator<char>());
            if (content.empty()) { warning = "empty fastresume: " + entry.path().string(); continue; }
            lt::error_code ec;
            auto params = lt::read_resume_data(lt::span<char const>(content.data(), content.size()), ec);
            if (ec) { warning = "invalid fastresume " + entry.path().string() + ": " + ec.message(); continue; }
            // A magnet's fastresume may not contain enough metadata after an
            // interrupted first run.  Reuse the .torrent captured when the
            // metadata alert arrived, matching legacy's restart behaviour.
            const auto torrent_path = entry.path().parent_path() / (entry.path().stem().string() + ".torrent");
            if (std::filesystem::is_regular_file(torrent_path)) {
                lt::error_code torrent_ec;
                auto info = std::make_shared<lt::torrent_info>(torrent_path.string(), torrent_ec);
                if (!torrent_ec) params.ti = std::move(info);
            }
            session->session.add_torrent(std::move(params), ec);
            if (ec) { warning = "cannot restore " + entry.path().string() + ": " + ec.message(); continue; }
            ++restored;
        }
        if (!warning.empty()) set_error(error, error_size, warning);
        return restored;
    } catch (const std::exception& exception) {
        set_error(error, error_size, exception.what());
    }
    return 0;
}

int rextto_lt_save_resume(rextto_lt_session* session, const char* state_dir, char* error, size_t error_size) {
    if (session == nullptr || state_dir == nullptr) { set_error(error, error_size, "invalid resume save parameters"); return 0; }
    try {
        const std::filesystem::path directory(state_dir);
        std::filesystem::create_directories(directory);
        std::unordered_set<std::string> pending;
        std::unordered_set<std::string> present;
        for (const auto& handle : session->session.get_torrents()) {
            if (!handle.is_valid()) continue;
            const auto hash = hex_hash(handle);
            pending.insert(hash);
            present.insert(hash);
            // NOTE: do not request flush_disk_cache here: on shutdown it makes
            // libtorrent allocate/flush the whole disk cache and can fail with
            // std::bad_alloc for large sessions. The regular shutdown flush is
            // enough to persist progress.
            handle.save_resume_data(lt::torrent_handle::save_info_dict);
        }
        // Rimuove i resume dei torrent non più in sessione: senza questa pulizia
        // un torrent rimosso (o fallito) verrebbe ripristinato al riavvio.
        std::error_code iterate_ec;
        for (const auto& entry : std::filesystem::directory_iterator(directory, iterate_ec)) {
            if (iterate_ec) break;
            if (!entry.is_regular_file()) continue;
            const auto extension = entry.path().extension();
            if (extension != ".fastresume" && extension != ".torrent") continue;
            if (present.find(entry.path().stem().string()) != present.end()) continue;
            std::error_code remove_ec;
            std::filesystem::remove(entry.path(), remove_ec);
        }
        const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(10);
        std::string failure;
        while (!pending.empty() && std::chrono::steady_clock::now() < deadline) {
            session->session.wait_for_alert(std::chrono::milliseconds(250));
            std::vector<lt::alert*> alerts;
            session->session.pop_alerts(&alerts);
            for (auto* alert : alerts) {
                if (const auto* saved = lt::alert_cast<lt::save_resume_data_alert>(alert)) {
                    const auto hash = hex_hash(saved->handle);
                    if (!pending.erase(hash)) continue;
                    // Per-torrent guard: a single bad handle must not abort the
                    // whole shutdown save.
                    try {
                        auto content = lt::write_resume_data_buf(saved->params);
                        std::string write_error;
                        if (!write_atomic(directory / (hash + ".fastresume"), content, write_error)) failure = "cannot save fastresume " + hash + ": " + write_error;
                    } catch (const std::exception& exception) {
                        failure = "cannot encode fastresume " + hash + ": " + exception.what();
                    }
                } else if (const auto* failed = lt::alert_cast<lt::save_resume_data_failed_alert>(alert)) {
                    pending.erase(hex_hash(failed->handle));
                    failure = "libtorrent fastresume failed: " + failed->error.message();
                }
            }
        }
        if (!pending.empty() && failure.empty()) failure = "timed out waiting for fastresume alerts";
        if (!failure.empty()) { set_error(error, error_size, failure); return 0; }
        return 1;
    } catch (const std::exception& exception) {
        set_error(error, error_size, exception.what());
    } catch (...) {
        set_error(error, error_size, "unknown fastresume error");
    }
    return 0;
}
}
