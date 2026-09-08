//! Stress-corpus integration test per `SPECIFICATION.md` §8.3 / §8.4.
//!
//! Parses the inline stress cases and asserts zero `ERROR`
//! and zero `MISSING` nodes for every case. The `MISSING`-node check
//! is the direct anti-regression for `dekobon/big-code-analysis#246`,
//! which surfaced because the prior grammar inserted a synthetic
//! missing operand into elvis-chain parses.

use tree_sitter::{Node, Parser};

// Synthetic project snippets or adaptations of public-domain examples.
// Dual-licensed Apache-2.0 OR MIT, like the grammar.
const STRESS_CASES: &[(&str, &str)] = &[
    (
        "arithmetic_and_ranges.groovy",
        r###"def a = 1 + 2 * 3
def b = (a + 1) * (a - 1)
def c = 2 ** 8
def d = 0xFFFFL
def e = 1_000_000.0
def f = 1.5e-7

def lo = 0
def hi = 10
def inclusive = lo..hi
def exclusive_right = lo..<hi
def exclusive_left = lo<..hi
def exclusive_both = lo<..<hi

def sum = a + b + c + d + e + f
def diff = a - b
def power = (a + 1) ** 2

def shifted = a << 2
def masked = 0xFFFF & 0x00FF
"###,
    ),
    (
        "class_with_methods.groovy",
        r###"package com.example

class Point {
    def distance(Point other) {
        def dx = x - other.x
        def dy = y - other.y
        Math.sqrt(dx * dx + dy * dy)
    }

    def translated(int delta = 1) {
        new Point(x + delta, y + delta)
    }

    def equals(Object other) {
        if (other instanceof Point) {
            x == other.x && y == other.y
        } else {
            false
        }
    }
}

trait Named {
    def name() {
        "unknown"
    }
}

interface Shape {
    def area()
    def perimeter()
}

@interface Stable {
    def since()
}
"###,
    ),
    (
        "closures_and_lists.groovy",
        r###"def numbers = [1, 2, 3, 4, 5]
def doubled = numbers.collect({ n -> n * 2 })
def filtered = numbers.findAll({ n -> n > 2 })

def lookup = [
    one: 1,
    two: 2,
    three: 3,
]

def composed = [
    *: lookup,
    four: 4,
]

def nested = [
    [1, 2, 3],
    [4, 5, 6],
    [7, 8, 9],
]

def operations = [
    add: { a, b -> a + b },
    sub: { a, b -> a - b },
    mul: { a, b -> a * b },
]

def callOp(map, op, a, b) {
    map[op](a, b)
}

def spread_call(xs) {
    operations.add(*xs)
}

println 'starting'
print numbers
debug { numbers.size() }
log 'done'
"###,
    ),
    (
        "control_flow.groovy",
        r###"def classify(x) {
    if (x < 0) {
        return 'negative'
    } else if (x == 0) {
        return 'zero'
    } else {
        return 'positive'
    }
}

def sumUp(xs) {
    def total = 0
    for (n in xs) {
        total = total + n
    }
    return total
}

def countDown(n) {
    while (n > 0) {
        n = n - 1
    }
    return n
}

def describe(c) {
    switch (c) {
        case 1 -> 'one'
        case 2 -> 'two'
        case 3 -> 'three'
        default -> 'many'
    }
}

def safeRun(action) {
    try {
        action()
    } catch (IllegalStateException | IllegalArgumentException e) {
        e.message
    } catch (Throwable t) {
        'unknown'
    } finally {
        action()
    }
}

def withResource() {
    try (def r = open()) {
        r.use()
    }
}

outer: for (i in 0..10) {
    inner: for (j in 0..10) {
        if (i + j > 15) {
            break outer
        }
    }
}
"###,
    ),
    (
        "generics.groovy",
        r###"// Stress coverage for SPECIFICATION.md §4 / §5.14 generics.
// Exercises generic_type, type_arguments, type_parameters,
// method_type_parameters, and the wildcard variants in every
// position the grammar currently accepts a `_type`.

import java.util.List
import java.util.Map
import java.util.ArrayList

class Box<T> {
    List<T> contents = []

    def get(T value) { return value }
}

class Pair<A, B> {
    A first = null
    B second = null

    def make(A a, B b) { return [a, b] }
}

interface Holder<T extends Comparable<T>> {
}

trait Boxable<T> {
}

class Util {
    static <T> T identity(T x) {
        return x
    }

    static <T extends Number & Comparable> T pick(T a) {
        return a
    }

    static <K, V> Map<K, V> emptyMap() {
        return [:]
    }
}

def items = new ArrayList<String>()
List<String> names = []
Map<String, Integer> counts = [:]
Map<String, List<Integer>> nested = [:]
List<? extends Number> nums = []
List<? super Integer> sup = []
List<?> any = []
java.util.List<String> qual = []

def cast = (List<String>) other
def ax = obj as List<String>

def closure = { List<String> xs -> xs }
"###,
    ),
    (
        "gradle_buildscript.groovy",
        r###"// Synthetic Gradle build script. Exercises the closure-as-DSL
// pattern (`plugins`, `repositories`, `dependencies`, `tasks`),
// command-chain method invocation (`id 'java'`,
// `implementation '…'`), and `register` / configuration DSL.

plugins {
    id 'java'
    id 'application'
    id 'groovy'
}

group = 'com.example'
version = '0.1.0'

repositories {
    mavenCentral()
    gradlePluginPortal()
}

dependencies {
    implementation 'com.google.guava:guava:30.1.1-jre'
    implementation 'org.codehaus.groovy:groovy-all:3.0.10'
    testImplementation 'org.spockframework:spock-core:2.0-groovy-3.0'
    testImplementation 'junit:junit:4.13.2'
}

application {
    mainClass = 'com.example.Main'
}

tasks.withType(JavaCompile) {
    options.encoding = 'UTF-8'
}

tasks.named('test') {
    useJUnitPlatform()
    testLogging {
        events 'passed', 'failed', 'skipped'
    }
}
"###,
    ),
    (
        "imports_and_package.groovy",
        r###"package com.example.demo

import java.util.List
import java.util.Map
import java.util.concurrent.atomic.AtomicLong
import java.util.*
import static java.util.Collections.unmodifiableMap
import static java.lang.Math.*
import com.example.legacy.Helper as OldHelper

class Demo {
    def value() {
        42
    }
}
"###,
    ),
    (
        "jenkins_pipeline.groovy",
        r###"// Realistic Jenkinsfile shape — `pipeline` block from
// `SPECIFICATION.md` §4 / §10 row #37 wrapping ordinary closure
// DSL invocations. Zero ERROR / MISSING is the integration-test
// contract.

pipeline {
    agent any

    environment {
        FOO = "bar"
        DEPLOY_TARGET = "staging"
    }

    stages {
        stage('Build') {
            steps {
                sh 'make build'
                archiveArtifacts artifacts: 'build/**/*'
            }
        }

        stage('Test') {
            steps {
                sh 'make test'
            }
            post {
                always {
                    junit 'build/test-results/**/*.xml'
                }
            }
        }

        stage('Deploy') {
            when {
                branch 'main'
            }
            steps {
                sh "deploy.sh ${DEPLOY_TARGET}"
            }
        }
    }

    post {
        always {
            cleanWs()
        }
        failure {
            mail(to: 'team@example.com', subject: "Build failed", body: "see logs")
        }
    }
}
"###,
    ),
    (
        "operators_grab_bag.groovy",
        r###"def a = 5
def b = 3

def cmp = a <=> b
def identity = a === b
def diff = a !== b

def isOne = a == 1
def isLess = a < b
def isOdd = a % 2 == 1

def picked = a ?: b
def ternary = a > b ? a : b
def assertion = a >= 0
def implication = a > 0 ==> b > 0

def matched = "hello" =~ "[a-z]+"
def fullmatch = "abc" ==~ "^a.c$"

def safe = a?.toString()
def safeChain = a??.toString()
def methodPtr = a.&toString
def directField = a.@value
def methodRef = String::length
def spreadProp = [1, 2, 3]*.toString()

def updated = a++
def prefixed = ++b

def assigned = a
assigned = b
assigned += 1
assigned -= 1
assigned *= 2
assigned /= 2
assigned %= 3
assigned **= 2
assigned <<= 1
assigned >>= 1
assigned >>>= 1
assigned &= 0xFF
assigned ^= 0x0F
assigned |= 0xF0
assigned ?= b
"###,
    ),
];

/// Walks `node` and the entire subtree, returning the first node it
/// finds whose kind is `ERROR` or whose `is_missing()` flag is set.
fn find_first_problem(node: Node<'_>) -> Option<Node<'_>> {
    if node.is_error() || node.is_missing() {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_first_problem(child) {
            return Some(found);
        }
    }
    None
}

#[test]
fn parses_stress_corpus_with_no_errors() {
    let mut parser = Parser::new();
    parser
        .set_language(&dekobon_tree_sitter_groovy::LANGUAGE.into())
        .expect("load Groovy grammar");

    let mut failures = Vec::new();
    for &(name, source) in STRESS_CASES {
        let tree = parser
            .parse(source, None)
            .unwrap_or_else(|| panic!("parser returned None for {name}"));
        if let Some(problem) = find_first_problem(tree.root_node()) {
            let kind = if problem.is_missing() {
                format!("MISSING {}", problem.kind())
            } else {
                "ERROR".to_string()
            };
            failures.push(format!(
                "{}:{}:{}: {}",
                name,
                problem.start_position().row + 1,
                problem.start_position().column + 1,
                kind,
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "stress corpus has {} parse failure(s):\n{}",
        failures.len(),
        failures.join("\n"),
    );
}
