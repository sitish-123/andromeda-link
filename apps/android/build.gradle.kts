plugins {
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.android.library) apply false
    alias(libs.plugins.kotlin.android) apply false
    alias(libs.plugins.kotlin.compose) apply false
    alias(libs.plugins.ktlint) apply false
}

subprojects {
    apply(plugin = "org.jlleitschuh.gradle.ktlint")
    dependencies { add("ktlintRuleset", rootProject.libs.compose.rules.ktlint) }
    configure<org.jlleitschuh.gradle.ktlint.KtlintExtension> {
        filter { exclude { it.file.path.contains("/build/") } }
    }
}
