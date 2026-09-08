class Box<T> { }
//    ^ type.definition
//        ^ type

List<String> items = []
// <- type
//   ^ type

class Holder<T extends Comparable<T>> { }
//    ^ type.definition
//           ^ type
//             ^ keyword
//                     ^ type

class Util { static <T> T pick(T x) { x } }
//    ^ type.definition
//                   ^ type
//                      ^ type
//                             ^ type
