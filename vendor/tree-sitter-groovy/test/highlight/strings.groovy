'hello'
// <- string

'it\'s a test'
// <- string

'''triple'''
// <- string

'''
multi-line
string'''
// <- string

"hi ${name}"
// <- string
//  ^ punctuation.special
//        ^ punctuation.special

"value $count"
//     ^ variable

"""triple ${expr}"""
// <- string
//        ^ punctuation.special
