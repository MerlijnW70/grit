#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::Host;
use grit::credit::{
    self, Board, Complements, Marker, Twin, applied, counted_singles, credited, escape, inserted,
    inserts, paired, repaired, repairs, singles,
};
use grit::netlist::{Exam, Gate, Netlist, drawn};
use grit::serve::{CROWD, Effort, Escape, Escapes, checked, handle, sought};
use grit::wire::{Json, read, written};

const FIXTURES: &str = r"
22 1 1056 294 1115136 4195591,1049613,4194574,7341320,4194569,11536141,2099209,6294273,10485774,7340545,12582925,3147522,18875650,9440512,17829132,16779777,5245190,23073031,1794,15731721,13637638,18880006
22 2 1056 64 1115136 5243398,3147526,3146497,4195592,10486793,11534598,10487299,1051917,8391947,7343885,1033,15729926,7343368,17830413,10490893,12588290,13636107,24121608,10485761,14686217,19928582,18876678
22 5 1056 176 1115136 4195597,7340043,1049871,6292487,5243395,8388873,11536130,12584206,1049606,15732232,16777731,1048591,2100493,13635341,16779021,2102022,3149060,19928843,13631489,8393737,22025737,25168649
22 6 1056 235 1115136 5242893,4195597,1049606,4196105,7341064,2061,8389896,13632270,13632008,2098441,15732745,13633794,10488072,5245188,12587529,15730951,19924743,5246473,3329,22025731,19927302,15734793
22 10 1056 198 1115136 5243401,5243657,1049606,8390665,4195592,4194310,1051654,8391438,13632769,15728902,12584194,7342855,3148293,11538191,11536130,9441291,4199943,11537677,11534337,17831945,23074054,14684678
22 11 1056 191 1115136 4195592,7340551,5242884,5243147,1049609,9437446,9438729,2098443,7343118,7340814,6291462,3849,18875662,7340558,5246985,11536903,23073544,10489869,1793,18875401,20977414,22022150
22 12 1056 211 1115136 3146249,4194310,4194569,5243915,5243911,11534851,10486281,8391681,10487051,10489355,12582923,11537668,1051915,10487819,11536136,16779534,12583169,7342850,11534340,20977417,18880006,25169158
22 13 1056 315 1115136 5243398,5243653,4195597,9438470,3147526,5242882,9437195,4197385,10488583,10487563,1049609,17829377,17828365,15730702,11538693,13636104,3843,11534596,10485764,19926281,18878470,2101001
22 15 1056 155 1115136 4194566,1,5243910,5242893,5245455,4196873,8389895,5243401,7343112,4195591,16780802,13633286,5243654,19927048,18878477,15733507,14684422,22020867,16777220,6150,24121097,17830926
22 17 1056 262 1115136 2098438,4194569,5243912,5243654,4196877,9438466,1291,4197641,9439246,8392194,9438977,9438984,3073,14684428,1052673,12583173,1052674,9439758,2305,19926790,17829897,18880514
22 18 1056 167 1115136 1048582,4195587,5242884,1049613,1050633,2099718,8389890,13632008,1284,10486795,7343364,3149057,11538184,9439241,3145731,14681353,14684684,19926022,3329,20975878,23074822,18880006
22 20 1056 177 1115136 4195592,5243654,1805,4194310,7340558,8389384,11536137,262,4194568,5246731,7341572,10487823,10489869,8390404,2098951,9437451,10489348,16783115,2306,19928582,25168646,22025227
23 1 1127 262 1270129 3146249,3145743,5243912,1049871,9438984,1049606,9437698,1049602,14683398,2100238,1049601,6293006,9438470,3079,5248002,15733257,4878,9440269,3147017,2305,25171721,15734025,26217225
23 2 1127 281 1270129 1049606,3147521,2056,5242884,5243911,1291,7342598,12583945,11534856,3148551,11537672,7341070,8390409,2099982,15729926,5246985,5243908,16777480,7343624,11534340,17825798,26219782,23073801
23 3 1127 284 1270129 5242882,5243912,1049609,8389894,4196102,11536394,9439233,2100226,12583694,2099719,15732228,2561,6293519,19923211,11539462,11536651,23071241,5244423,2103049,2049,18877705,26217993,17828358
23 4 1127 220 1270129 4194566,7340040,2098438,4195592,10487811,4195597,11534861,1050381,5244930,12582923,11535112,14683913,13636102,10487554,20975622,1048838,13635844,4196354,24120577,11534340,14685450,9442057,24120326
23 6 1127 302 1270129 5242893,5244674,4195592,9437966,1048836,2098184,4194569,1051915,8391945,9441026,12584194,17826310,7344135,7341569,7340294,22022159,22023435,6291713,2317,26214406,26218500,24121865,17828358
23 7 1127 363 1270129 2098438,5242884,4195592,7342350,3146497,9437966,1049609,2100233,8389381,13633545,4197124,1284,16779531,18874633,4197387,20974859,10488331,24117250,2100747,9437185,19922953,10491401,14682633
23 11 1127 306 1270129 4195591,4194569,5243915,8389645,8388610,2317,9439240,8391944,5245697,2098958,7342084,7340552,3147277,15733254,3146502,3148294,3147533,23072271,24121351,7340036,12587270,20974086,9443590
23 12 1127 223 1270129 3146249,1049606,3146756,5243911,2055,8388877,3147523,10486280,5245961,5245447,10487553,4197897,10487816,4098,11538187,22020622,4621,4107,17829894,10485764,19928073,18879753,13637894
23 14 1127 200 1270129 2098441,1048585,4195592,5243394,9438989,5242893,1281,9437453,8390149,1051910,1049609,5244932,12586505,9437953,19927300,18876171,20972041,4198923,10486018,9437185,16783366,11539718,11540230
23 15 1127 236 1270129 1284,4196105,1050125,5243911,1049606,10486275,2098441,3148296,14681350,8391426,12586251,5243908,17828612,13634056,17829388,5244172,12585741,22024457,4609,2562,26220294,20975625,22023945
23 16 1127 277 1270129 4195586,4195592,1050624,1048580,4194307,1792,5242882,5243398,13631750,3146249,16779268,7340046,13635849,5245443,1051401,14682126,22022148,15729670,22023947,2049,24121862,23075081,17829385
23 17 1127 224 1270129 4194819,4195592,5244683,1293,1287,2099207,1050882,1798,1050374,14682889,2100236,10489351,783,7341583,9437449,16781070,9437963,5245959,2103302,9437188,13636105,26220038,25171721
23 18 1127 268 1270129 1282,4195592,1291,2099207,7340294,3147790,12585479,1049870,4196872,8391425,15728902,3148303,4196614,19927298,8390919,4194313,1797,18876167,2102281,2049,23072774,5249289,13632777
23 19 1127 146 1270129 1038,4195591,1049861,9437956,2099462,8389639,2311,9437962,3148553,12583177,4197641,17829895,16779272,11536397,9437958,7341325,9438471,4870,8392461,8388612,23074825,18879494,22026502
24 1 1200 259 1440000 1030,4195591,3146505,2098438,7341323,8389387,7341313,8388622,10487809,1050632,12586506,12586763,14684171,11536390,2097156,20972036,4198155,12583181,1054470,26219533,14680068,19926281,27265545,5247497
24 3 1200 292 1440000 2097155,5243912,7340806,4194564,7342084,5244939,12582916,1049609,14681345,11536905,15728647,10487566,3149829,12584967,8392193,9439234,23071757,17829897,1051657,3152140,2049,13636873,12589065,3151622
24 5 1200 291 1440000 1026,269,5243398,4195592,5244673,10488065,5243913,11536139,7341059,11534598,13631496,7344390,9437961,10489857,14682374,20975885,19925506,24118022,18878468,11534601,10485761,20975878,26217481,25167110
24 7 1200 237 1440000 4195592,2098944,5243404,4196621,2099466,2098693,7340289,7340551,1281,5246215,2098945,7340814,4194569,18875654,13635334,7340036,22021129,19928334,18877194,16781574,5641,23074561,27269129,14685193
24 8 1200 216 1440000 5243915,1030,1049860,7342084,9438217,2099212,5245707,3147526,13635083,4197386,3147268,13635592,7344646,4198412,17829633,3341,1054217,13631748,16780548,11540226,5637,10491910,19929609,26218253
24 10 1200 303 1440000 1291,4196105,4195586,5243401,9438466,1049609,8391431,1050374,3149069,12585742,9437185,1051394,2101263,2099976,3330,15729929,8393477,12584962,5244685,20974082,22026508,16781574,27269129,20977165
24 12 1200 289 1440000 5242891,1050374,1033,3147014,4195592,1049606,11534596,10488590,8388615,3148549,9440258,2101256,2099981,17828104,10489358,13636110,6294285,7343618,5247246,16779272,4098,27265286,23075078,19926537
24 14 1200 235 1440000 4195078,5243401,5243910,5243912,10487053,7341580,2099719,1049609,5242882,15729673,12585222,10487310,10489358,15732485,11534340,1288,23072769,16780801,17828363,25167878,2561,22024966,27266310,13637895
24 16 1200 303 1440000 6292741,4194569,1033,7342347,1793,1051401,7341058,2100482,13634054,2098950,3149060,8392462,17827081,10487303,12586244,14681865,5245954,10486790,16780548,17827340,3329,9442566,26219014,19926529
24 17 1200 320 1440000 4195593,1048581,7341320,1050377,10486528,5244930,1051654,8390925,10489095,11535882,9440514,7341316,2098178,17829385,13631491,19924237,3148036,4196874,1050882,2102793,2305,4199433,27266825,24122889
24 19 1200 326 1440000 5243912,4195597,4194566,9438222,2098952,9437191,9438980,7340802,12584717,10486278,14682121,16780046,18878982,16780545,14680587,4199690,12584203,12584962,18874633,2061,7340033,13638150,26220297,11538694
25 1 1275 279 1625625 2098183,3145989,5242893,3147017,1049858,5243912,6293767,12585486,12585728,14681103,2100235,5243908,2101511,17827078,15731979,8389638,12584964,1050886,23068681,25171467,26219012,3073,11541254,20978182,19926534
25 8 1275 324 1625625 3147017,5243401,5243912,2099458,9438470,10486025,6293508,5243913,5242881,9438989,4194563,11538701,15731721,14683142,20976392,22022402,2827,24120838,3150087,14680837,12585217,2305,25172742,8393993,16779780
25 9 1275 360 1625625 4194568,4195592,3147777,4195586,5244169,1030,9438470,6291968,12585486,4194566,15731211,17829895,8388865,5243398,20976137,8393739,12583683,1053711,22022145,13634050,15733504,8388609,19926793,16783625,23072007
25 11 1275 470 1625625 1049606,4195592,1284,8389390,2097674,2098441,8391693,4194306,8389897,15728651,8390413,17829897,4196614,19923468,18876163,10489097,3149824,19924743,23071748,12588034,256,2049,1053194,25169161,2102793
25 12 1275 489 1625625 1049609,5243912,3146249,1293,7342601,8388609,5243398,5245953,9439243,4197897,5245953,16779009,13633537,10488331,5244685,2102535,16781570,775,9438728,11536398,4200454,16780298,27262985,19927561,15731974
25 13 1275 370 1625625 1049609,4195592,8389894,8389389,8390411,2099201,12585741,9440013,8389644,7343622,3147528,5245449,12583181,14683142,2315,22021129,3148558,10487041,4196618,23072779,2099202,8388609,16782598,27268102,18881284
25 14 1275 369 1625625 6291717,5243915,5244936,1284,9437954,7341065,7343105,3147785,2099464,2099209,9440258,10486793,10489856,19926020,4198146,4363,8391430,2050,23071491,18877442,16779534,2305,25170185,28318214,24121099
25 15 1275 371 1625625 1291,1049606,1281,5243911,8390918,11536904,2097929,2098441,11535369,10485774,5243914,9439748,5244937,5242886,10489090,5248265,6293764,19926798,10489348,25169160,4199172,16777222,12587010,25172233,23069449
25 16 1275 363 1625625 4195586,7340033,3147017,2050,1048582,4194569,9437705,12585742,5244683,9438476,13635587,14683914,8392974,7343629,20972038,13635339,22023938,23071246,1052430,18875396,8395017,3842,12589830,24123654,23070982
25 17 1275 257 1625625 1287,7340034,5243401,1050127,1049609,3148301,5243658,5242884,1052166,5243912,16777998,9441288,15732737,4194313,18878733,4100,5248262,18876674,7345158,15730178,26217227,4097,19928070,25172742,10491655
25 18 1275 326 1625625 5243657,4194566,4195592,4194313,9437454,9438989,12585728,5245441,14682382,13634830,5243398,9441550,12587521,9437707,5245190,1030,7345413,23070985,22025742,15730695,1053190,9437185,11540742,18881033,12588041
25 20 1275 420 1625625 5242882,3146249,2098441,7340046,5243915,7341065,5245963,8389639,13633800,5244678,9439243,3146508,4194566,7343367,19926279,11534592,14681351,12587778,2829,19923718,23070984,3330,22026505,25169673,3151625
26 1 1352 416 1827904 3147017,4194564,4195591,3145990,5243401,774,2099467,8390147,3145736,1049606,5242891,1284,17825794,19923975,13634312,20972300,18874630,7342349,24121346,16781577,11534340,9443848,2306,7174,22026505,25169159
26 4 1352 399 1827904 522,1049606,5243912,4195586,4194823,2307,12584193,12585989,1051398,9437966,2099469,17829896,4197638,3150596,2099470,5245709,14685708,9439245,22022662,19925000,12585476,25167373,12585220,28317702,26221062,18876934
26 5 1352 411 1827904 5242885,1284,5243912,7341065,5245188,2098438,1050894,1050630,1049606,5246469,10488577,3148809,11536142,1051397,9441803,22024706,18875407,5248264,1051914,12585230,15732235,15733512,9437185,13635849,28318217,3151878
26 12 1352 413 1827904 4194566,7340043,5243912,3147014,6293765,9437707,9438991,2305,7342347,8388873,2099470,7341321,2100745,9439758,18878477,5243908,18877448,23068686,1054730,6291456,17831174,9439488,14682124,25169673,5249801,20974600
26 16 1352 468 1827904 1282,5243654,4195592,4196097,5243401,11536651,4194569,5242882,9439234,9437704,3149583,9438732,5245455,2569,14680326,2097675,22024196,13636865,4612,9437454,24118531,3151875,2305,20978182,12589062,15732750
26 20 1352 433 1827904 4195591,5242881,3146240,3147014,262,1049609,2098945,2099211,2097417,10487565,7343876,5245709,16778503,18875661,12584969,8389903,7342850,12587524,4200192,25171206,18880518,4200705,1794,24122633,17832457,16780548
27 1 1431 588 2047761 1281,7341065,265,1050888,5245195,8389890,5244934,8391437,8389122,15729922,4194566,9440780,12584206,3148814,2101254,13635847,20976896,1051648,7344640,22022404,23074319,10489355,20972290,3073,1052166,23074057,16782342
27 6 1431 487 2047761 2098180,5242882,5243911,3147790,5245193,2817,2099470,8389641,4194566,1050882,9438469,2099464,15732228,9437960,4195847,7341066,19927561,23068676,4198406,2817,1052160,20972809,8394249,2306,26217478,5248777,29363458
27 7 1431 455 2047761 1294,4194824,8389901,2098433,262,9440004,5243914,9437700,4195592,15731726,2099462,4196365,1796,4194566,6291976,14684427,22023681,16782343,4196105,26214409,14683648,3149582,15731458,15728641,30415366,25170182,29362441
27 11 1431 526 2047761 5244167,4194569,4195597,7340040,4196870,7341058,7341065,1030,12585740,4198156,2099975,8391694,4196097,14680326,17827595,14680579,3148801,13634060,11536395,4865,14683918,9439243,2103558,3073,27267590,22027526,22025990
27 12 1431 425 2047761 4195075,3147017,1050375,9437702,5243912,9440005,7341323,2099970,9440775,1291,11536386,2817,2823,17830406,12583182,10490886,13631750,16783113,14682630,25168651,16783111,11536906,23074816,18876172,19929606,28317958,17829385
27 15 1431 501 2047761 5243912,7340814,1539,2098951,5243918,9438465,5245704,11537419,13633286,12583942,4194566,17827595,16778758,13633545,10487047,15728654,12582920,14681602,16781575,4364,2103558,10486790,6294541,7342338,23073289,22027014,22022153
27 17 1431 459 2047761 4194820,4196099,4194313,5243908,5243912,4194566,8390155,2099982,11537922,7341325,5243151,3149830,15730434,17829638,10488066,10488846,5245197,11535631,24122888,23069957,11540493,11534593,11535111,2817,29365510,26217990,18881798
27 18 1431 435 2047761 5243398,5242894,265,1050369,2097157,5243911,1049606,3147014,3146249,4197903,8391942,7340807,10489348,5245186,15731716,10490632,16782080,24122635,12584715,12585217,20974605,18878476,13638407,11537416,27267334,26221833,22023686
27 20 1431 510 2047761 265,5242894,8389899,4196614,8389641,10487042,12583694,2099725,4196358,7343106,14683656,1049609,10490369,9441545,12583432,16779529,16782607,5248260,3086,14685952,16780045,19923465,13634573,12582913,28311814,29366281,25169158
28 1 1512 583 2286144 1049606,3146759,4195592,2099469,3148036,9438466,3148042,9438978,12582926,10487561,2097408,12587022,15729670,13631745,15732229,18874886,14685709,8390411,19927299,14684939,8394496,26221317,2099982,11535625,9437185,15734025,27268617,31459842
28 2 1512 574 2286144 4195592,2098438,5243657,9437966,1049609,7342094,1048581,1284,2098946,2100238,5246215,777,4197894,17826060,12586250,11538694,20977166,22024195,2098955,11539202,7345677,27264772,8391951,7342338,7343362,28311561,12589574,31461121
28 3 1512 608 2286144 4194824,2097152,7342083,1792,4195592,1049606,5245700,1284,14683139,14681097,10489100,7341325,12585731,13631489,9441288,6293511,23070976,24122372,11537410,3148545,1052165,16780301,2101769,19928323,2817,20977929,30415881,27267590
28 4 1512 509 2286144 1048843,265,7341317,3148038,4194566,5242891,3145731,11537414,8390659,11536391,4195591,2101517,10490120,5243659,11538702,22024456,17829387,9441288,15733249,2101773,20977411,22024713,6293249,20971789,4354,5897,29366281,19929606
28 5 1512 583 2286144 2097929,1036,4195592,7341064,4195597,10487053,12583430,8390661,9437185,15729921,11535877,1052681,11534347,15731471,2099462,4194569,16779784,1053198,2573,9440514,9442830,8392716,24118792,18880011,15729420,19929865,31463942,7343366
28 7 1512 575 2286144 5243912,2098441,1049606,7342093,5244677,5242882,2097929,13633288,14681346,3146503,11534337,4197385,11534600,18876685,17830670,18876160,5243392,11538178,9440780,4197633,22025728,18874635,5247501,29366274,2818,22027526,20974089,3149574
28 9 1512 626 2286144 5243654,5242893,4195591,2317,2099460,9438983,10487042,11537417,4194569,6294539,15731977,5243401,11538694,8389888,9441544,8389641,2100231,23071749,1051402,19928583,17828879,6295041,11534336,3152911,10486275,22020102,18881030,14683404
28 16 1512 578 2286144 5243913,4196104,1281,2050,7340297,6293504,9440262,1050886,2097929,14680074,2098438,8390926,14682126,13635333,4199174,4196357,15730692,20975364,23073031,11539207,17831430,4199177,29361412,2097158,23068676,10492934,26221062,17831689
";

struct Fixture {
    gates: u32,
    seed: u32,
    singles: u64,
    neutral: u64,
    pairs: u64,
    circuit: Vec<u32>,
}

fn fixtures() -> Vec<Fixture> {
    FIXTURES
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            Fixture {
                gates: parts[0].parse().unwrap(),
                seed: parts[1].parse().unwrap(),
                singles: parts[2].parse().unwrap(),
                neutral: parts[3].parse().unwrap(),
                pairs: parts[4].parse().unwrap(),
                circuit: parts[5]
                    .split(',')
                    .map(|one| one.parse().unwrap())
                    .collect(),
            }
        })
        .collect()
}

fn alu() -> Vec<u32> {
    (0..128_u32)
        .map(|w| {
            let d = w & 15;
            match (w >> 4) & 3 {
                0 => (d + 1) & 15,
                1 => d.wrapping_sub(1) & 15,
                2 => !d & 15,
                _ => (d << 1) & 15,
            }
        })
        .collect()
}

fn alu_exam(gates: u32) -> (Netlist, Exam) {
    let shape = Netlist::over(7, 4, gates, 16).unwrap();
    (shape, Exam::of(shape, alu()).unwrap())
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }

    fn below(&mut self, most: u32) -> u32 {
        self.next() % most.max(1)
    }
}

fn random_problem(rng: &mut Rng) -> (Netlist, Exam) {
    let inputs = 1 + rng.below(7);
    let outputs = 1 + rng.below(4);
    let gates = outputs + rng.below(31 - outputs);
    let shape = Netlist::over(inputs, outputs, gates, 16).unwrap();
    let table = (0..shape.words())
        .map(|_| rng.next() & ((1 << outputs) - 1))
        .collect();
    (shape, Exam::of(shape, table).unwrap())
}

fn random_chip(shape: Netlist, rng: &mut Rng) -> Vec<u32> {
    (0..shape.gates)
        .map(|at| drawn(&shape, at, rng.next()).pack())
        .collect()
}

fn checked_marks(shape: Netlist, exam: &Exam, circuit: &[u32]) -> i64 {
    i64::from(checked(shape, exam, circuit).unwrap().marks)
}

fn every_single_agrees(
    shape: Netlist,
    exam: &Exam,
    circuit: &[u32],
    tangent: &credit::Tangent,
) -> usize {
    let board = Board::of(exam);
    let base = checked_marks(shape, exam, circuit);
    assert_eq!(i64::from(tangent.marks), base);
    let mut seen = 0;
    for change in singles(shape, circuit) {
        let predicted = board.credit(tangent, circuit, change);
        let real = checked_marks(shape, exam, &applied(circuit, change)) - base;
        assert_eq!(
            predicted, real,
            "{change:?} of {circuit:?}: credit {predicted}, checked {real}"
        );
        seen += 1;
    }
    seen
}

fn tool(host: Option<&Host>, arguments: &str) -> Json {
    let asked = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{{\"name\":\"grit_credit\",\"arguments\":{arguments}}}}}"
    );
    let answer = handle(host, &read(&asked).unwrap()).unwrap();
    answer
        .get("result")
        .and_then(|one| one.get("structuredContent"))
        .cloned()
        .unwrap_or_else(|| panic!("{}", written(&answer)))
}

fn refused(arguments: &str) -> Json {
    let asked = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{{\"name\":\"grit_credit\",\"arguments\":{arguments}}}}}"
    );
    let answer = handle(None, &read(&asked).unwrap()).unwrap();
    assert!(
        answer.get("error").is_some()
            || answer
                .get("result")
                .and_then(|one| one.get("isError"))
                .is_some_and(|one| *one == Json::Bool(true)),
        "{}",
        written(&answer)
    );
    answer
}

fn chip_json(circuit: &[u32]) -> String {
    let gates: Vec<String> = circuit
        .iter()
        .map(|one| {
            let gate = Gate::unpack(*one);
            format!(
                "{{\"kind\":{},\"left\":{},\"right\":{}}}",
                gate.kind, gate.left, gate.right
            )
        })
        .collect();
    format!("[{}]", gates.join(","))
}

fn arguments(shape: Netlist, exam: &Exam, circuit: &[u32], more: &str) -> String {
    let table: Vec<String> = (0..shape.words())
        .map(|row| exam.asked(row).to_string())
        .collect();
    format!(
        "{{\"inputs\":{},\"outputs\":{},\"gates\":{},\"table\":[{}],\"chip\":{}{more}}}",
        shape.inputs,
        shape.outputs,
        shape.gates,
        table.join(","),
        chip_json(circuit)
    )
}

fn host_for_test(label: &str) -> Option<Host> {
    device(label)
}

fn field_of(name: &str) -> u32 {
    credit::FIELDS.iter().position(|one| *one == name).unwrap() as u32
}

#[test]
fn every_single_credit_equals_the_checked_marks() {
    let host = host_for_test("every_single_credit_equals_the_checked_marks");
    let mut total = 0;
    for one in fixtures() {
        let (shape, exam) = alu_exam(one.gates);
        let answer = tool(
            host.as_ref(),
            &arguments(shape, &exam, &one.circuit, ",\"depth\":1"),
        );
        let base = checked_marks(shape, &exam, &one.circuit);
        assert_eq!(i64::from(answer.counted("marks").unwrap()), base);
        let listed = answer.get("changes").unwrap().list().unwrap();
        assert_eq!(listed.len() as u64, one.singles);
        assert_eq!(
            answer.get("singles").unwrap().int().unwrap() as u64,
            one.singles
        );
        let mut neutral = 0;
        for change in listed {
            let change_at = credit::Change {
                at: change.counted("gate").unwrap(),
                field: field_of(change.get("field").unwrap().text().unwrap()),
                value: change.counted("value").unwrap(),
            };
            let said = change.get("credit").unwrap().int().unwrap();
            let real = checked_marks(shape, &exam, &applied(&one.circuit, change_at)) - base;
            assert_eq!(
                said, real,
                "ALU {} seed {}: {change_at:?} credited {said}, checked {real}",
                one.gates, one.seed
            );
            neutral += u64::from(said == 0);
            total += 1;
        }
        assert_eq!(
            neutral, one.neutral,
            "ALU {} seed {}: neutral singles against the research's count",
            one.gates, one.seed
        );
        assert_eq!(answer.counted("improving_singles"), Some(0));
    }
    assert_eq!(total, 90_037);
    let mut rng = Rng(0x0dd_c0ffee);
    let mut chips = 0;
    let mut changes = 0;
    while chips < 1000 {
        let (shape, exam) = random_problem(&mut rng);
        let board = Board::of(&exam);
        let circuits: Vec<Vec<u32>> = (0..50).map(|_| random_chip(shape, &mut rng)).collect();
        let tangents = match &host {
            Some(host) => {
                let room = credit::room(shape, CROWD);
                let mut resident = credit::opened(host, &board, room).unwrap();
                let mut marker = Marker::within(&mut resident, 1, 0, host.width(), room, shape);
                marker.tangents(&board, &circuits).unwrap()
            }
            None => Twin.tangents(&board, &circuits).unwrap(),
        };
        for (circuit, tangent) in circuits.iter().zip(&tangents) {
            changes += every_single_agrees(shape, &exam, circuit, tangent);
            chips += 1;
        }
    }
    eprintln!("72 fixtures, 90037 changes; {chips} random chips, {changes} changes; 0 mismatches");
}

#[test]
fn pair_credits_equal_brute_force_and_count_the_square_of_the_singles() {
    let host = host_for_test("pair_credits_equal_brute_force_and_count_the_square_of_the_singles");
    let mut rng = Rng(0x5eed_0002);
    let mut sampled = 0;
    for one in fixtures() {
        let (shape, exam) = alu_exam(one.gates);
        let board = Board::of(&exam);
        let base = checked_marks(shape, &exam, &one.circuit);
        let firsts = singles(shape, &one.circuit);
        let picks: Vec<(credit::Change, Vec<u32>)> = (0..2000)
            .map(|_| {
                let first = firsts[rng.below(firsts.len() as u32) as usize];
                (first, applied(&one.circuit, first))
            })
            .collect();
        let mids: Vec<Vec<u32>> = picks.iter().map(|(_, mid)| mid.clone()).collect();
        let check = |source: &mut dyn FnMut(&[Vec<u32>]) -> Vec<credit::Tangent>, rng: &mut Rng| {
            let tangents = source(&mids);
            for ((_, mid), tangent) in picks.iter().zip(&tangents) {
                let seconds = singles(shape, mid);
                let second = seconds[rng.below(seconds.len() as u32) as usize];
                let predicted =
                    i64::from(tangent.marks) - base + board.credit(tangent, mid, second);
                let real = checked_marks(shape, &exam, &applied(mid, second)) - base;
                assert_eq!(predicted, real, "ALU {} seed {}", one.gates, one.seed);
            }
        };
        let found = match &host {
            Some(host) => {
                let room = credit::room(shape, CROWD);
                let mut resident = credit::opened(host, &board, room).unwrap();
                let mut marker = Marker::within(&mut resident, 1, 0, host.width(), room, shape);
                check(&mut |mids| marker.tangents(&board, mids).unwrap(), &mut rng);
                paired(
                    &board,
                    &mut marker,
                    &one.circuit,
                    credit::PAIRS,
                    credit::PAIRS_LISTED,
                )
                .unwrap()
            }
            None => {
                check(&mut |mids| Twin.tangents(&board, mids).unwrap(), &mut rng);
                paired(
                    &board,
                    &mut Twin,
                    &one.circuit,
                    credit::PAIRS,
                    credit::PAIRS_LISTED,
                )
                .unwrap()
            }
        };
        sampled += 2000;
        assert_eq!(found.total, one.pairs);
        assert_eq!(found.evaluated, one.pairs);
        assert_eq!(found.total, one.singles * one.singles);
        assert!(
            found.best <= 0,
            "ALU {} seed {}: {}",
            one.gates,
            one.seed,
            found.best
        );
        assert_eq!(found.improving, 0);
        for (gain, first, second) in &found.top {
            let real = checked_marks(
                shape,
                &exam,
                &applied(&applied(&one.circuit, *first), *second),
            ) - base;
            assert_eq!(*gain, real);
        }
    }
    eprintln!("{sampled} sampled pairs, 0 mismatches");
}

#[test]
fn the_device_answer_equals_the_host_twin_bit_for_bit() {
    let Some(host) = device("the_device_answer_equals_the_host_twin_bit_for_bit") else {
        return;
    };
    let mut rng = Rng(0xb17_b17);
    for _ in 0..20 {
        let (shape, exam) = random_problem(&mut rng);
        let board = Board::of(&exam);
        let circuits: Vec<Vec<u32>> = (0..50).map(|_| random_chip(shape, &mut rng)).collect();
        let room = credit::room(shape, CROWD);
        let mut resident = credit::opened(&host, &board, room).unwrap();
        let mut marker = Marker::within(&mut resident, 1, 0, host.width(), room, shape);
        assert_eq!(
            marker.tangents(&board, &circuits).unwrap(),
            Twin.tangents(&board, &circuits).unwrap(),
            "{shape:?}"
        );
    }
    for one in fixtures().iter().step_by(9) {
        let (shape, exam) = alu_exam(one.gates);
        let asked = arguments(shape, &exam, &one.circuit, "");
        assert_eq!(
            written(&tool(Some(&host), &asked)),
            written(&tool(None, &asked)),
            "ALU {} seed {}",
            one.gates,
            one.seed
        );
    }
}

#[test]
fn a_complement_batch_wider_than_the_device_room_is_marked_in_parts() {
    let Some(host) = device("a_complement_batch_wider_than_the_device_room_is_marked_in_parts")
    else {
        return;
    };
    let mut rng = Rng(0x0070_0064);
    let shape = Netlist::over(7, 4, 70, 16).unwrap();
    let table = (0..shape.words()).map(|_| rng.next() & 15).collect();
    let exam = Exam::of(shape, table).unwrap();
    let board = Board::of(&exam);
    let circuits: Vec<Vec<u32>> = (0..3).map(|_| random_chip(shape, &mut rng)).collect();
    let width = host.width();
    for room in [width, 64.max(width), 71_u32.div_ceil(width) * width, 213] {
        let mut resident = credit::opened(&host, &board, room).unwrap();
        let mut marker = Marker::within(&mut resident, 1, 0, width, room, shape);
        assert_eq!(
            marker.tangents(&board, &circuits).unwrap(),
            Twin.tangents(&board, &circuits).unwrap(),
            "room {room}"
        );
        assert!(marker.spent.most <= room, "{:?}", marker.spent);
    }
}

#[test]
fn a_crowd_smaller_than_one_complement_batch_still_escapes() {
    let Some(host) = device("a_crowd_smaller_than_one_complement_batch_still_escapes") else {
        return;
    };
    let mut rng = Rng(12345);
    let shape = Netlist::over(7, 4, 70, 16).unwrap();
    let table: Vec<u32> = (0..shape.words()).map(|_| rng.next() & 15).collect();
    let exam = Exam::of(shape, table.clone()).unwrap();
    let effort = Effort {
        seed: 1,
        stall: 2,
        escape: Escape::Credit,
        ..Effort::of(64, 3000)
    };
    let once = sought(&host, shape, &exam, effort, 0).unwrap();
    let again = sought(&host, shape, &exam, effort, 0).unwrap();
    assert!(once.escapes.ran > 0, "{:?}", once.escapes);
    assert_eq!(
        (once.marks, once.rounds, &once.gates, once.escapes),
        (again.marks, again.rounds, &again.gates, again.escapes)
    );
    let rows: Vec<String> = table.iter().map(u32::to_string).collect();
    for name in ["grit_synthesise", "grit_least"] {
        let asked = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{{\"inputs\":7,\"outputs\":4,\"gates\":70,\"table\":[{}],\"crowd\":64,\"rounds\":3000,\"stall\":2,\"seed\":1,\"escape\":\"credit\"}}}}}}",
            rows.join(",")
        );
        let answer = handle(Some(&host), &read(&asked).unwrap()).unwrap();
        let result = answer.get("result").unwrap();
        assert_eq!(
            result.get("isError"),
            Some(&Json::Bool(false)),
            "{name}: {}",
            written(&answer)
        );
        let said = written(result.get("structuredContent").unwrap());
        let ran: Vec<u64> = said
            .match_indices("\"ran\":")
            .map(|(at, key)| {
                said[at + key.len()..]
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
                    .parse()
                    .unwrap()
            })
            .collect();
        eprintln!("{name}: escapes ran {ran:?}");
        assert!(ran.iter().any(|one| *one > 0), "{name}: {said}");
    }
}

#[test]
fn every_stuck_fixture_is_certified_two_by_count() {
    let host = host_for_test("every_stuck_fixture_is_certified_two_by_count");
    for one in fixtures() {
        let (shape, exam) = alu_exam(one.gates);
        let answer = tool(host.as_ref(), &arguments(shape, &exam, &one.circuit, ""));
        assert_eq!(
            answer.counted("certified"),
            Some(2),
            "ALU {} seed {}",
            one.gates,
            one.seed
        );
        let by = answer.get("certified_by").unwrap();
        assert_eq!(
            by.get("singles").unwrap().int().unwrap() as u64,
            one.singles
        );
        assert_eq!(by.get("pairs").unwrap().int().unwrap() as u64, one.pairs);
        let pairs = answer.get("pairs").unwrap();
        assert_eq!(pairs.get("complete"), Some(&Json::Bool(true)));
        assert_eq!(pairs.get("improving").unwrap().int(), Some(0));
    }
}

#[test]
fn a_pair_budget_short_of_every_pair_certifies_one_and_says_so() {
    let one = &fixtures()[0];
    let (shape, exam) = alu_exam(one.gates);
    let short = one.pairs - 1;
    let answer = tool(
        None,
        &arguments(shape, &exam, &one.circuit, &format!(",\"pairs\":{short}")),
    );
    assert_eq!(answer.counted("certified"), Some(1));
    let pairs = answer.get("pairs").unwrap();
    assert_eq!(pairs.get("complete"), Some(&Json::Bool(false)));
    let evaluated = pairs.get("evaluated").unwrap().int().unwrap() as u64;
    assert_eq!(evaluated, (short / one.singles) * one.singles);
    assert_eq!(
        answer
            .get("certified_by")
            .unwrap()
            .get("pairs")
            .unwrap()
            .int()
            .unwrap() as u64,
        evaluated
    );
    let single = tool(None, &arguments(shape, &exam, &one.circuit, ",\"depth\":1"));
    assert_eq!(single.counted("certified"), Some(1));
    assert!(single.get("pairs").is_none());
}

#[test]
fn a_change_that_gains_certifies_nothing() {
    let (shape, exam) = alu_exam(22);
    let board = Board::of(&exam);
    let mut rng = Rng(0x1234_9876);
    let circuit = random_chip(shape, &mut rng);
    let tangent = board.tangent(&circuit);
    let found = credited(&board, &tangent, &circuit);
    assert!(found.improving > 0);
    let answer = tool(None, &arguments(shape, &exam, &circuit, ",\"depth\":1"));
    assert_eq!(answer.counted("certified"), Some(0));
    assert_eq!(
        answer.get("improving_singles").unwrap().int().unwrap() as u64,
        found.improving
    );
    let first = &answer.get("changes").unwrap().list().unwrap()[0];
    assert_eq!(first.get("credit").unwrap().int(), Some(found.best));
}

#[test]
fn every_insert_and_repair_credit_equals_the_checked_marks() {
    let mut lifted = Vec::new();
    for one in fixtures() {
        let (shape, exam) = alu_exam(one.gates);
        let board = Board::of(&exam);
        let tangent = board.tangent(&one.circuit);
        let base = checked_marks(shape, &exam, &one.circuit);
        let insert = inserts(&board, &tangent, &one.circuit, credit::MOVES);
        let repair = repairs(&board, &tangent, &one.circuit, credit::MOVES);
        assert!(insert.complete && repair.complete);
        if let Some((gain, move_)) = insert.best {
            let real = checked_marks(shape, &exam, &inserted(shape, &one.circuit, move_)) - base;
            assert_eq!(gain, real, "insert {move_:?}");
        }
        if let Some((gain, move_)) = repair.best {
            let real = checked_marks(shape, &exam, &repaired(shape, &one.circuit, move_)) - base;
            assert_eq!(gain, real, "repair {move_:?}");
        }
        if insert.improving > 0 || repair.improving > 0 {
            lifted.push((
                one.gates,
                one.seed,
                insert.best.map(|b| b.0),
                repair.best.map(|b| b.0),
            ));
        }
    }
    assert_eq!(lifted, vec![(28, 4, Some(4), Some(4))]);
    let mut rng = Rng(0xfeed_1234);
    for _ in 0..200 {
        let (shape, exam) = random_problem(&mut rng);
        let board = Board::of(&exam);
        let circuit = random_chip(shape, &mut rng);
        let tangent = board.tangent(&circuit);
        let base = checked_marks(shape, &exam, &circuit);
        if let Some((gain, move_)) = inserts(&board, &tangent, &circuit, credit::MOVES).best {
            assert_eq!(
                gain,
                checked_marks(shape, &exam, &inserted(shape, &circuit, move_)) - base
            );
        }
        if let Some((gain, move_)) = repairs(&board, &tangent, &circuit, credit::MOVES).best {
            assert_eq!(
                gain,
                checked_marks(shape, &exam, &repaired(shape, &circuit, move_)) - base
            );
        }
    }
}

#[test]
fn the_escape_lifts_the_one_fixture_a_dead_gate_can_lift() {
    let all = fixtures();
    let one = all
        .iter()
        .find(|one| one.gates == 28 && one.seed == 4)
        .unwrap();
    let (shape, exam) = alu_exam(one.gates);
    let board = Board::of(&exam);
    let out = escape(
        &board,
        &mut Twin,
        &one.circuit,
        u64::from(CROWD) * credit::ESCAPE_ROUNDS,
    )
    .unwrap();
    assert!(out.marks >= 508, "{out:?}");
    assert_eq!(
        i64::from(out.marks),
        checked_marks(shape, &exam, &out.circuit)
    );
    assert_eq!(
        out.steps.first().map(|one| one.0),
        Some(credit::Step::Insert)
    );
    let stuck = all
        .iter()
        .find(|one| one.gates == 22 && one.seed == 1)
        .unwrap();
    let (shape, exam) = alu_exam(stuck.gates);
    let board = Board::of(&exam);
    let budget = u64::from(CROWD) * credit::ESCAPE_ROUNDS;
    let out = escape(&board, &mut Twin, &stuck.circuit, budget).unwrap();
    assert_eq!(out.marks, 504);
    assert!(out.steps.is_empty());
    assert!(out.evaluated <= budget);
    let again = escape(&board, &mut Twin, &stuck.circuit, budget).unwrap();
    assert_eq!(out, again);
    let _ = counted_singles(shape);
}

#[test]
fn a_credit_call_replays_from_its_arguments() {
    let host = host_for_test("a_credit_call_replays_from_its_arguments");
    let one = &fixtures()[5];
    let (shape, exam) = alu_exam(one.gates);
    let asked = arguments(shape, &exam, &one.circuit, "");
    let first = written(&tool(host.as_ref(), &asked));
    let second = written(&tool(host.as_ref(), &asked));
    assert_eq!(first, second);
}

fn rows() -> String {
    alu()
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn synthesised(host: &Host, more: &str) -> String {
    let asked = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"grit_synthesise\",\"arguments\":{{\"inputs\":7,\"outputs\":4,\"gates\":22,\"table\":[{}],\"crowd\":2048,\"rounds\":1200,\"stall\":60{more}}}}}}}",
        rows()
    );
    let answer = handle(Some(host), &read(&asked).unwrap()).unwrap();
    let held = answer
        .get("result")
        .and_then(|one| one.get("structuredContent"))
        .unwrap_or_else(|| panic!("{}", written(&answer)));
    written(held)
}

#[test]
fn a_search_with_the_credit_escape_replays_from_its_seed_and_shake_is_the_default() {
    let Some(host) =
        device("a_search_with_the_credit_escape_replays_from_its_seed_and_shake_is_the_default")
    else {
        return;
    };
    let plain = synthesised(&host, "");
    let shaken = synthesised(&host, ",\"escape\":\"shake\"");
    assert_eq!(plain, shaken);
    assert!(!plain.contains("escapes"));
    let credit = synthesised(&host, ",\"escape\":\"credit\"");
    assert!(credit.contains("\"escapes\""), "{credit}");
    let (shape, exam) = alu_exam(22);
    let run = |escape: Escape| {
        let effort = Effort {
            seed: 1,
            stall: 100,
            escape,
            ..Effort::of(4096, 3000)
        };
        sought(&host, shape, &exam, effort, 0).unwrap()
    };
    let once = run(Escape::Credit);
    let again = run(Escape::Credit);
    assert!(
        once.escapes.ran > 0 && once.escapes.lifted > 0,
        "{:?}",
        once.escapes
    );
    assert_eq!(
        (once.marks, once.rounds, &once.gates, once.escapes),
        (again.marks, again.rounds, &again.gates, again.escapes)
    );
    let shaken = run(Escape::Shake);
    assert_eq!(shaken.escapes, Escapes::default());
    let packed: Vec<u32> = once.gates.iter().map(|one| one.pack()).collect();
    assert_eq!(i64::from(once.marks), checked_marks(shape, &exam, &packed));
    eprintln!(
        "credit: marks {} rounds {} escapes {:?}; shake: marks {} rounds {}",
        once.marks, once.rounds, once.escapes, shaken.marks, shaken.rounds
    );
}

fn listed_tool(name: &str) -> Json {
    let asked = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}";
    let answer = handle(None, &read(asked).unwrap()).unwrap();
    answer
        .get("result")
        .and_then(|one| one.get("tools"))
        .and_then(Json::list)
        .unwrap()
        .iter()
        .find(|one| one.get("name").and_then(Json::text) == Some(name))
        .cloned()
        .unwrap()
}

fn property(tool: &Json, key: &str) -> Json {
    tool.get("inputSchema")
        .and_then(|one| one.get("properties"))
        .and_then(|one| one.get(key))
        .cloned()
        .unwrap_or_else(|| panic!("{key} in {}", written(tool)))
}

#[test]
fn every_number_the_credit_tools_hold_is_published_in_their_schema() {
    let credit_tool = listed_tool("grit_credit");
    for (key, least, most, or) in [
        (
            "depth",
            1,
            i64::from(credit::DEPTH_MOST),
            i64::from(credit::DEPTH_MOST),
        ),
        ("pairs", 1, credit::PAIRS_MOST as i64, credit::PAIRS as i64),
        ("moves", 0, credit::MOVES_MOST as i64, credit::MOVES as i64),
        (
            "listed",
            0,
            i64::from(credit::LISTED),
            i64::from(credit::LISTED),
        ),
        ("work", 1, credit::WORK_MOST as i64, credit::WORK as i64),
    ] {
        let held = property(&credit_tool, key);
        assert_eq!(
            held.get("minimum").and_then(Json::int),
            Some(least),
            "{key}"
        );
        assert_eq!(held.get("maximum").and_then(Json::int), Some(most), "{key}");
        assert_eq!(held.get("default").and_then(Json::int), Some(or), "{key}");
    }
    for name in ["grit_synthesise", "grit_least"] {
        let held = property(&listed_tool(name), "escape");
        assert_eq!(held.get("default"), Some(&Json::Text("shake".to_owned())));
        assert_eq!(
            held.get("enum"),
            Some(&Json::List(vec![
                Json::Text("shake".to_owned()),
                Json::Text("credit".to_owned())
            ]))
        );
        let said = written(&held);
        assert!(said.contains(&credit::ESCAPE_ROUNDS.to_string()), "{said}");
    }
    let machine = listed_tool("grit_machine");
    assert!(
        machine
            .get("inputSchema")
            .and_then(|one| one.get("properties"))
            .and_then(|one| one.get("escape"))
            .is_none()
    );
}

fn told(answer: &Json, key: &str) -> Option<Json> {
    answer
        .get("result")
        .and_then(|one| one.get("structuredContent"))
        .and_then(|one| one.get(key))
        .cloned()
}

#[test]
fn a_credit_call_past_what_its_work_allows_is_refused_up_front_with_the_estimate() {
    let (inputs, outputs, gates) = (11_u32, 32_u32, 4085_u32);
    let table = vec!["0"; 1 << inputs].join(",");
    let answer = refused(&format!(
        "{{\"inputs\":{inputs},\"outputs\":{outputs},\"gates\":{gates},\"table\":[{table}],\"chip\":[]}}"
    ));
    let said = written(&answer);
    assert_eq!(
        told(&answer, "code"),
        Some(Json::Text("too_many".to_owned())),
        "{said}"
    );
    assert_eq!(
        told(&answer, "field"),
        Some(Json::Text("$.gates".to_owned())),
        "{said}"
    );
    let most = (outputs..gates)
        .rev()
        .find(|g| {
            credit::Costs::of(Netlist::over(inputs, outputs, *g, 16).unwrap()).least()
                <= credit::WORK_MOST
        })
        .unwrap();
    assert!(said.contains(&format!("at most {most} gates")), "{said}");
    assert_eq!(
        told(&answer, "limit"),
        Some(Json::Int(i64::from(most))),
        "{said}"
    );
    let (inputs, gates) = (12_u32, 3000_u32);
    let table = vec!["0"; 1 << inputs].join(",");
    let answer = refused(&format!(
        "{{\"inputs\":{inputs},\"outputs\":1,\"gates\":{gates},\"table\":[{table}],\"chip\":[]}}"
    ));
    let needed = credit::Costs::of(Netlist::over(inputs, 1, gates, 16).unwrap()).least();
    assert!(needed > credit::WORK && needed <= credit::WORK_MOST);
    assert_eq!(
        told(&answer, "field"),
        Some(Json::Text("$.work".to_owned()))
    );
    assert_eq!(told(&answer, "limit"), Some(Json::Int(needed as i64)));
    let one = &fixtures()[0];
    let (shape, exam) = alu_exam(one.gates);
    let least = credit::Costs::of(shape).least();
    let short = refused(&arguments(
        shape,
        &exam,
        &one.circuit,
        &format!(",\"work\":{}", least - 1),
    ));
    assert_eq!(told(&short, "field"), Some(Json::Text("$.work".to_owned())));
    assert_eq!(told(&short, "limit"), Some(Json::Int(least as i64)));
    let enough = tool(
        None,
        &arguments(shape, &exam, &one.circuit, &format!(",\"work\":{least}")),
    );
    assert_eq!(enough.counted("certified"), Some(1));
    let pairs = enough.get("pairs").unwrap();
    assert_eq!(pairs.get("complete"), Some(&Json::Bool(false)));
    assert_eq!(pairs.get("evaluated").unwrap().int(), Some(0));
    let work = enough.get("work").unwrap();
    assert_eq!(work.get("spent").unwrap().int(), Some(least as i64));
    let whole = tool(None, &arguments(shape, &exam, &one.circuit, ""));
    let work = whole.get("work").unwrap();
    assert!(work.get("spent").unwrap().int().unwrap() <= credit::WORK as i64);
    assert_eq!(whole.counted("certified"), Some(2));
    let deep = refused(
        "{\"inputs\":2,\"outputs\":1,\"gates\":1,\"table\":[0,1,1,0],\"chip\":[{\"kind\":6,\"left\":0,\"right\":1}],\"depth\":3}",
    );
    assert!(written(&deep).contains("$.depth"));
    let shape = Netlist::over(12, 4, 400, 16).unwrap();
    let mut rng = Rng(400);
    let table: Vec<u32> = (0..shape.words()).map(|_| rng.next() & 15).collect();
    let exam = Exam::of(shape, table).unwrap();
    let circuit = random_chip(shape, &mut rng);
    let wide = refused(&arguments(shape, &exam, &circuit, ",\"depth\":1"));
    assert_eq!(
        told(&wide, "code"),
        Some(Json::Text("answer_too_large".to_owned())),
        "{}",
        written(&wide)
    );
    assert_eq!(told(&wide, "field"), Some(Json::Text("$.gates".to_owned())));
}
