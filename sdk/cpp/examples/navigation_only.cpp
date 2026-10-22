#include <geotrace/geotrace.hpp>

#if GEOTRACE_CPP_EXCEPTIONS
#include <exception>
#endif
#include <iostream>

int main() {
#if GEOTRACE_CPP_EXCEPTIONS
    try {
#endif
        using geotrace::Angle;
        using geotrace::FileBuilder;
        using geotrace::FixTime;
        using geotrace::NavFix;
        using geotrace::SolutionState;
        using geotrace::Timestamp;

        NavFix fix{FixTime::receiver(Timestamp::from_seconds(1792670400)), Angle::degrees(55.67),
                   Angle::degrees(12.56)};
        fix.state = SolutionState::Fix;
        fix.used_count = 8;
        fix.hdop = 0.9;
        fix.heading = Angle::degrees(75);
        fix.speed = geotrace::Velocity::mps(4.2);
        FileBuilder builder;
        builder.title("Basic navigation recording");
        builder.add(fix);
        builder.add(NavFix::status_only(FixTime::receiver(Timestamp::from_seconds(1792670402)),
                                        SolutionState::NoFix));
        NavFix recovered{FixTime::receiver(Timestamp::from_seconds(1792670405)),
                         Angle::degrees(55.671), Angle::degrees(12.562)};
        recovered.state = SolutionState::Fix;
        recovered.used_count = 9;
        builder.add(recovered);
        const auto file = builder.try_finish();
        if (file.is_err()) {
            std::cerr << file.error().description << '\n';
            return 1;
        }
        const auto status = file.value().try_write_to_file("navigation_only.gtd");
        if (status.is_err()) {
            std::cerr << status.description << '\n';
            return 1;
        }
        return 0;
#if GEOTRACE_CPP_EXCEPTIONS
    } catch (const std::exception &error) {
        std::cerr << error.what() << '\n';
        return 1;
    }
#endif
}
